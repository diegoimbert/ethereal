//! Device → UI analysis channel, controller side (v0.2, implemented and frozen by
//! contracts-3; CONTRACTS.md §12.4.3). Producers: `fx-analysis`, `fx-dynamics`,
//! `racks-modulation` (engine side: `ether_core::analysis`).
//!
//! - `Analysis::{Watch, Unwatch}` maintain the watched-device set (runtime, not saved;
//!   cleared when a project is opened or closed).
//! - Every tick, [`EtherController::analysis_tick`] drains `EngineBridge::poll_analysis`,
//!   maps node keys to devices, keeps the **latest frame per (device, kind)** and emits one
//!   `Event::Analysis` per watched device and kind. Frames of unwatched devices are dropped.

use std::collections::{BTreeMap, BTreeSet};

use ether_core::analysis::{AnalysisFrame, AnalysisKind};
use ether_core::protocol::analysis::{
    AnalysisCommand, AnalysisData, AnalysisEvent, ModulatedValue,
};
use ether_core::protocol::model::{DeviceId, ParamId};
use ether_core::protocol::{Event, ReplyValue};

use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

#[derive(Default)]
pub(crate) struct AnalysisState {
    watched: BTreeSet<DeviceId>,
    frames: Vec<AnalysisFrame>,
}

impl AnalysisState {
    /// Forget every watch (project opened/closed).
    pub(crate) fn clear(&mut self) {
        self.watched.clear();
    }
}

fn kind_tag(k: AnalysisKind) -> u8 {
    match k {
        AnalysisKind::Spectrum => 0,
        AnalysisKind::Tuner => 1,
        AnalysisKind::Levels => 2,
        AnalysisKind::Modulation => 3,
    }
}

/// Decode an engine frame (encodings: `ether_core::analysis` module docs).
pub(crate) fn decode(frame: &AnalysisFrame) -> Option<AnalysisData> {
    let v = frame.values();
    Some(match frame.kind {
        AnalysisKind::Spectrum => {
            if v.len() < 2 {
                return None;
            }
            AnalysisData::Spectrum {
                min_hz: v[0],
                max_hz: v[1],
                bins_db: v[2..].to_vec(),
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
    pub(crate) fn analysis_command(&mut self, command: &AnalysisCommand) -> CmdResult<ReplyValue> {
        match command {
            AnalysisCommand::Watch { device } => {
                self.analysis.watched.insert(*device);
            }
            AnalysisCommand::Unwatch { device } => {
                self.analysis.watched.remove(device);
            }
        }
        Ok(ReplyValue::Unit)
    }

    /// Called every tick (see the module docs).
    pub(crate) fn analysis_tick(&mut self, out: &mut dyn MessageSink) {
        let mut frames = std::mem::take(&mut self.analysis.frames);
        frames.clear();
        self.bridge.poll_analysis(&mut frames);
        if !frames.is_empty() && !self.analysis.watched.is_empty() {
            let mut latest: BTreeMap<(DeviceId, u8), usize> = BTreeMap::new();
            for (i, f) in frames.iter().enumerate() {
                if let Some(device) = self.engine.device_of(f.node)
                    && self.analysis.watched.contains(&device)
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
        f.begin(AnalysisKind::Spectrum);
        for v in [20.0, 20000.0, -60.0, -50.0] {
            f.push(v);
        }
        assert!(
            matches!(decode(&f), Some(AnalysisData::Spectrum { bins_db, .. }) if bins_db.len() == 2)
        );
    }
}
