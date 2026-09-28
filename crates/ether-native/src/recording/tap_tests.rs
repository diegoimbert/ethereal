//! `tap-recording`: takes recorded from a track input tap (resampling) through the real
//! engine, capture ring and writer thread (offline backend, no input device).

use ether_core::graph::{ChainEntry, TrackDesc};
use ether_core::protocol::model::{BeatRange, InputTap};
use ether_core::{AudioBuffers, InputTapDesc, Node, PrepareConfig, ProcessContext, ProcessStatus};

use super::*;

/// The known signal at timeline sample `p` (left; the right is `-0.5 *` it).
fn known(p: i64) -> f32 {
    (p.rem_euclid(2001) * 7919 % 2001) as f32 / 1000.0 - 1.0
}

/// Plays [`known`] at the timeline position it renders (follows loops and locates).
struct Timeline;

impl Node for Timeline {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let p0 = (ctx.transport.position * SPB).round() as i64;
        let playing = ctx.transport.playing;
        let [l, r] = audio.outputs else {
            return ProcessStatus::Continue;
        };
        for (i, (l, r)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            let s = if playing { known(p0 + i as i64) } else { 0.0 };
            *l = s;
            *r = -0.5 * s;
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// A pure delay reporting its latency (a look-ahead device stand-in).
struct Latency {
    samples: usize,
    buf: [Vec<f32>; 2],
    pos: usize,
}

impl Node for Latency {
    fn prepare(&mut self, _: &PrepareConfig) {
        self.buf = [vec![0.0; self.samples], vec![0.0; self.samples]];
        self.pos = 0;
    }
    fn reset(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
    }
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let len = self.samples;
        let mut pos = self.pos;
        for ch in 0..2 {
            pos = self.pos;
            for i in 0..audio.outputs[ch].len() {
                let x = audio.inputs[ch][i];
                audio.outputs[ch][i] = self.buf[ch][pos];
                self.buf[ch][pos] = x;
                pos = (pos + 1) % len;
            }
        }
        self.pos = pos;
        ProcessStatus::Continue
    }
    fn latency(&self) -> u32 {
        self.samples as u32
    }
}

fn chain(handle: &mut EngineHandle, mut t: TrackDesc, nodes: Vec<Box<dyn Node>>) -> TrackDesc {
    for n in nodes {
        let node = handle.add_node(n).unwrap();
        t.chain.push(ChainEntry {
            node,
            enabled: true,
            sidechain: None,
        });
    }
    t
}

/// Master, a source playing [`known`] through 300 samples of latency, and an armed audio
/// track (not monitored) whose input is the source's `point` tap. Returns the consumer.
fn tap_graph(rig: &mut Rig, point: InputTap, loop_region: Option<(f64, f64)>) -> TrackId {
    let master = track(1, TrackKind::Master, None);
    let src = chain(
        &mut rig.handle,
        track(2, TrackKind::Audio, Some(master.id)),
        vec![
            Box::new(Timeline),
            Box::new(Latency {
                samples: 300,
                buf: [vec![], vec![]],
                pos: 0,
            }),
        ],
    );
    let mut rec = track(3, TrackKind::Audio, Some(master.id));
    rec.input_tap = Some(InputTapDesc {
        track: src.id,
        point,
    });
    rec.armed = true;
    let id = rec.id;
    let (s, e) = loop_region.unwrap_or((0.0, 0.0));
    rig.handle
        .publish(RenderGraphDesc {
            tracks: vec![master, src, rec],
            loop_enabled: loop_region.is_some(),
            loop_start: s,
            loop_end: e,
            ..Default::default()
        })
        .unwrap();
    if loop_region.is_some() {
        rig.handle
            .transport(TransportControl::SetLoop {
                enabled: true,
                region: BeatRange {
                    start: Beats(s),
                    end: Beats(e),
                },
            })
            .unwrap();
    }
    id
}

fn tap_session(project: ProjectId, track: TrackId) -> RecordSession {
    RecordSession {
        taps: vec![track],
        ..session(project, vec![], false)
    }
}

/// Compare a take (stereo, at `take.start`) with [`known`] frame by frame; returns the
/// number of frames compared.
fn assert_take_is_known(root: &std::path::Path, project: ProjectId, take: &AudioTake) -> usize {
    let (channels, rate, samples) = read_wav(&root.join(project.to_string()).join(&take.file));
    assert_eq!((channels, rate), (2, SR));
    assert_eq!(samples.len() as u64, take.frames * 2);
    let first = (take.start * SPB).round() as i64;
    for (j, frame) in samples.chunks(2).enumerate() {
        let expected = known(first + j as i64);
        assert!(
            (frame[0] - expected).abs() < 1e-6 && (frame[1] + 0.5 * expected).abs() < 1e-6,
            "take frame {j}: {frame:?} vs {expected}"
        );
    }
    samples.len() / 2
}

use ether_controller::AudioTake;

#[test]
fn a_tap_take_equals_the_source_after_pdc() {
    let tmp = TempDir::new("rec-tap");
    let mut rig = rig(AudioBackendKind::Null, None, tmp.path());
    let rec = tap_graph(&mut rig, InputTap::PostFader, None);
    let project = ProjectId::v7(1_750_000_000_000, [5; 10]);
    let takes = record(&mut rig, &tap_session(project, rec), 3.0);
    assert!(takes.warnings.is_empty(), "{:?}", takes.warnings);
    assert_eq!(takes.audio.len(), 1, "{takes:?}");
    let take = &takes.audio[0];
    assert_eq!(take.track, rec);
    assert_eq!((take.channels, take.sample_rate), (2, SR));
    assert!(
        take.start.abs() < 1e-9,
        "starts at the record position: {take:?}"
    );
    assert!(take.file.starts_with("media/rec-t-"), "{}", take.file);
    let n = assert_take_is_known(tmp.path(), project, take);
    assert!(n as f64 > 2.9 * SPB, "{n} frames");
}

#[test]
fn loop_recording_a_tap_makes_one_take_per_pass() {
    let tmp = TempDir::new("rec-tap-loop");
    let mut rig = rig(AudioBackendKind::Null, None, tmp.path());
    let rec = tap_graph(&mut rig, InputTap::PostFx, Some((1.0, 2.0)));
    let project = ProjectId::v7(1_750_000_000_000, [6; 10]);
    let s = RecordSession {
        keep_from: 1.0,
        ..tap_session(project, rec)
    };
    start(&rig.shared, &s).unwrap();
    for c in [
        TransportControl::Locate {
            position: Beats(1.0),
        },
        TransportControl::SetRecording { enabled: true },
        TransportControl::Play,
    ] {
        rig.handle.transport(c).unwrap();
    }
    // Two full passes, then stop half-way through the third.
    let mut wraps = 0;
    let mut last = 1.0;
    wait_for("2.5 passes", || {
        let p = rig.handle.playhead().position.0;
        if p < last - 0.5 && last < 2.0 && p >= 1.0 {
            wraps += 1;
        }
        last = p;
        wraps >= 2 && p > 1.5
    });
    rig.handle
        .transport(TransportControl::SetRecording { enabled: false })
        .unwrap();
    rig.handle.transport(TransportControl::Stop).unwrap();
    let takes = stop(&rig.shared, &rig.handle).unwrap();
    assert_eq!(takes.audio.len(), 3, "{takes:?}");
    for (i, take) in takes.audio.iter().enumerate() {
        assert_eq!(take.track, rec);
        assert!(
            (take.start - 1.0).abs() < 1e-9,
            "pass {i} starts at the loop start: {take:?}"
        );
        let n = assert_take_is_known(tmp.path(), project, take);
        if i < 2 {
            assert_eq!(n as f64, SPB, "a full pass is one loop region long");
        } else {
            assert!(n > 0 && (n as f64) < SPB);
        }
    }
}

#[test]
fn hardware_and_tap_takes_record_together() {
    let tmp = TempDir::new("rec-tap-hw");
    let mut rig = rig(AudioBackendKind::Null, Some("loopback:1000"), tmp.path());
    let rec = tap_graph(&mut rig, InputTap::PostFader, None);
    // A second armed track records the loopback input (hardware).
    let project = ProjectId::v7(1_750_000_000_000, [8; 10]);
    let hw = TrackId(Ulid(4));
    let s = RecordSession {
        taps: vec![rec],
        ..session(
            project,
            vec![AudioTarget {
                track: hw,
                first: 0,
                count: 1,
            }],
            false,
        )
    };
    let takes = record(&mut rig, &s, 2.0);
    let tracks: Vec<TrackId> = takes.audio.iter().map(|t| t.track).collect();
    assert_eq!(tracks.len(), 2, "{takes:?}");
    assert!(tracks.contains(&rec) && tracks.contains(&hw));
    let tap = takes.audio.iter().find(|t| t.track == rec).unwrap();
    assert!(tap.start.abs() < 1e-9);
    assert_take_is_known(tmp.path(), project, tap);
    // The loopback hears the whole mix (compensated by the round trip): the take is placed
    // at the record position too, a different file.
    let hw_take = takes.audio.iter().find(|t| t.track == hw).unwrap();
    assert!(hw_take.start.abs() < 1e-9);
    assert_ne!(hw_take.file, tap.file);
    assert_eq!(hw_take.channels, 1);
}
