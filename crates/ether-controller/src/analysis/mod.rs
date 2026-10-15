//! Device → UI analysis channel, controller side (v0.2, implemented and frozen by
//! contracts-3; CONTRACTS.md §12.4.3). Producers: `fx-analysis`, `fx-dynamics`,
//! `graphical-eq`, `racks-modulation` (engine side: `ether_core::analysis`).
//!
//! - Watches are **refcounted**: every `Analysis::Watch` adds one, every `Unwatch` removes
//!   one. Connections own their watches: the remote server's router sends an `Unwatch` for
//!   each watch a disconnecting client still held (`ether-server/src/router.rs`). Project
//!   loads keep them (a watch of a device that no longer exists is inert), so the router's
//!   per-client counts stay exact; single-connection hosts call
//!   [`EtherController::reset_analysis_watches`] when their UI reconnects.
//! - Each tick, [`EtherController::analysis_tick`] tells the engine which nodes to collect
//!   (`EngineBridge::watch_analysis`, diffed against what it sent; node re-creation changes
//!   keys and is picked up the same way; a failed push stays pending and is retried next
//!   tick), drains `EngineBridge::poll_analysis`, maps node keys
//!   to devices, keeps the **latest frame per (device, kind)** and emits one
//!   `Event::Analysis` per watched device and kind.

use std::collections::{BTreeMap, BTreeSet};

use ether_core::NodeKey;
use ether_core::analysis::{AnalysisFrame, AnalysisKind};
use ether_core::protocol::analysis::{
    AnalysisCommand, AnalysisData, AnalysisEvent, ModulatedValue, SpectrumStage,
};
use ether_core::protocol::model::{DeviceId, ParamId};
use ether_core::protocol::{Event, ReplyValue};

use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

#[derive(Default)]
pub(crate) struct AnalysisState {
    /// Device → number of watches.
    watched: BTreeMap<DeviceId, u32>,
    /// Node keys the engine was told to collect.
    sent: BTreeSet<NodeKey>,
    frames: Vec<AnalysisFrame>,
}

impl AnalysisState {
    /// Current refcount of `device` (tests, diagnostics).
    #[cfg(test)]
    pub(crate) fn count(&self, device: DeviceId) -> u32 {
        self.watched.get(&device).copied().unwrap_or(0)
    }
}

fn kind_tag(k: AnalysisKind) -> u8 {
    match k {
        AnalysisKind::Spectrum => 0,
        AnalysisKind::Tuner => 1,
        AnalysisKind::Levels => 2,
        AnalysisKind::Modulation => 3,
        AnalysisKind::SpectrumPre => 4,
    }
}

/// Decode an engine frame (encodings: `ether_core::analysis` module docs).
pub(crate) fn decode(frame: &AnalysisFrame) -> Option<AnalysisData> {
    let v = frame.values();
    Some(match frame.kind {
        AnalysisKind::Spectrum | AnalysisKind::SpectrumPre => {
            if v.len() < 2 {
                return None;
            }
            AnalysisData::Spectrum {
                min_hz: v[0],
                max_hz: v[1],
                bins_db: v[2..].to_vec(),
                stage: if frame.kind == AnalysisKind::SpectrumPre {
                    SpectrumStage::Pre
                } else {
                    SpectrumStage::Post
                },
            }
        }
        AnalysisKind::Tuner => {
            if v.len() < 5 {
                return None;
            }
            AnalysisData::Tuner {
                hz: (v[0] > 0.0).then_some(v[0]),
                note: (v[1] >= 0.0).then(|| v[1].round().clamp(0.0, 127.0) as u8),
                cents: v[2],
                confidence: v[3],
                level_db: v[4],
            }
        }
        AnalysisKind::Levels => AnalysisData::Levels { values: v.to_vec() },
        AnalysisKind::Modulation => AnalysisData::Modulation {
            values: v
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| ModulatedValue {
                    param: ParamId(c[0].to_bits()),
                    base: c[1],
                    value: c[2],
                })
                .collect(),
        },
    })
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Forget every analysis watch. For single-connection hosts when their UI reconnects
    /// (a reload leaves the old UI's watches behind); the engine follows at the next tick.
    pub fn reset_analysis_watches(&mut self) {
        self.analysis.watched.clear();
    }

    pub(crate) fn analysis_command(&mut self, command: &AnalysisCommand) -> CmdResult<ReplyValue> {
        let w = &mut self.analysis.watched;
        match command {
            AnalysisCommand::Watch { device } => *w.entry(*device).or_insert(0) += 1,
            AnalysisCommand::Unwatch { device } => {
                if let Some(n) = w.get_mut(device) {
                    *n -= 1;
                    if *n == 0 {
                        w.remove(device);
                    }
                }
            }
        }
        Ok(ReplyValue::Unit)
    }

    /// Called every tick (see the module docs).
    pub(crate) fn analysis_tick(&mut self, out: &mut dyn MessageSink) {
        // Engine-side watches follow the device watches (and node re-creation).
        let want: BTreeSet<NodeKey> = self
            .analysis
            .watched
            .keys()
            .filter_map(|d| self.engine.node(*d))
            .collect();
        if want != self.analysis.sent {
            // Only what the engine accepted counts as sent; failures retry next tick.
            let stale: Vec<NodeKey> = self.analysis.sent.difference(&want).copied().collect();
            for k in stale {
                if self.bridge.watch_analysis(k, false).is_ok() {
                    self.analysis.sent.remove(&k);
                }
            }
            let fresh: Vec<NodeKey> = want.difference(&self.analysis.sent).copied().collect();
            for k in fresh {
                if self.bridge.watch_analysis(k, true).is_ok() {
                    self.analysis.sent.insert(k);
                }
            }
        }
        let mut frames = std::mem::take(&mut self.analysis.frames);
        frames.clear();
        self.bridge.poll_analysis(&mut frames);
        if !frames.is_empty() && !self.analysis.watched.is_empty() {
            let mut latest: BTreeMap<(DeviceId, u8), usize> = BTreeMap::new();
            for (i, f) in frames.iter().enumerate() {
                if let Some(device) = self.engine.device_of(f.node)
                    && self.analysis.watched.contains_key(&device)
                {
                    latest.insert((device, kind_tag(f.kind)), i);
                }
            }
            for ((device, _), i) in latest {
                if let Some(data) = decode(&frames[i]) {
                    event(
                        out,
                        Event::Analysis {
                            event: AnalysisEvent::Frame { device, data },
                        },
                    );
                }
            }
        }
        self.analysis.frames = frames;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_every_kind() {
        let mut f = AnalysisFrame::EMPTY;
        f.begin(AnalysisKind::Tuner);
        for v in [440.0, 69.0, -3.5, 0.9, -12.0] {
            f.push(v);
        }
        assert_eq!(
            decode(&f),
            Some(AnalysisData::Tuner {
                hz: Some(440.0),
                note: Some(69),
                cents: -3.5,
                confidence: 0.9,
                level_db: -12.0
            })
        );
        f.begin(AnalysisKind::Modulation);
        for v in [f32::from_bits(7), 0.25, 0.5] {
            f.push(v);
        }
        assert_eq!(
            decode(&f),
            Some(AnalysisData::Modulation {
                values: vec![ModulatedValue {
                    param: ParamId(7),
                    base: 0.25,
                    value: 0.5
                }]
            })
        );
        f.begin(AnalysisKind::SpectrumPre);
        for v in [20.0, 20000.0, -60.0, -50.0] {
            f.push(v);
        }
        assert!(matches!(
            decode(&f),
            Some(AnalysisData::Spectrum { bins_db, stage: SpectrumStage::Pre, .. }) if bins_db.len() == 2
        ));
    }

    #[test]
    fn watches_are_refcounted() {
        let mut s = AnalysisState::default();
        let d = DeviceId(ether_core::protocol::model::Ulid(1));
        *s.watched.entry(d).or_insert(0) += 2;
        assert_eq!(s.count(d), 2);
    }
}
