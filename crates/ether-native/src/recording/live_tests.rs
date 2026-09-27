//! Live recording view (`live-record`): the writer driven with synthetic capture blocks.

use ether_controller::{AudioTarget, RecordSession, RecordedTakes};
use ether_core::protocol::model::{ProjectId, TrackId, Ulid};
use ether_core::protocol::recording::{LiveAudioChunk, LiveMidiNote};
use ether_core::recording::{CaptureBlock, RecordedMidi as EngineMidi};

use super::live::{FRAMES_PER_PEAK, LiveShared, MAX_CHUNKS};
use super::writer::{Session, StartConfig};
use crate::test_util::TempDir;

const SR: u32 = 48_000;
const BLOCK: u32 = 256;
/// Beats per sample at 120 bpm.
const BPS: f64 = 2.0 / SR as f64;
const CHANNELS: usize = 2;

struct Rig {
    tmp: TempDir,
    project: ProjectId,
    live: LiveShared,
    session: Option<Session>,
    sample_time: u64,
    chunks: Vec<LiveAudioChunk>,
    notes: Vec<LiveMidiNote>,
}

fn target(track: u128, first: u16, count: u16) -> AudioTarget {
    AudioTarget {
        track: TrackId(Ulid(track)),
        first,
        count,
    }
}

impl Rig {
    fn new(name: &str, audio: Vec<AudioTarget>, keep: (f64, Option<f64>), latency: u64) -> Self {
        let tmp = TempDir::new(name);
        let project = ProjectId::v7(1_750_000_000_000, [3; 10]);
        let media_dir = tmp.path().join(project.to_string()).join("media");
        std::fs::create_dir_all(&media_dir).unwrap();
        let live = LiveShared::default();
        let config = StartConfig {
            session: RecordSession {
                project,
                tag: "live".into(),
                audio,
                midi: true,
                keep_from: keep.0,
                keep_until: keep.1,
            },
            media_dir,
            latency,
            midi_latency: 0,
            sample_rate: SR,
        };
        let session = Session::new(config, CHANNELS, live.clone());
        Self {
            tmp,
            project,
            live,
            session: Some(session),
            sample_time: 0,
            chunks: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// One engine block rendered at song `position` (synthetic, deterministic input).
    fn block(&mut self, position: f64) {
        let b = CaptureBlock {
            sample_time: self.sample_time,
            frames: BLOCK,
            position,
            beats_per_sample: BPS,
            dropped: false,
        };
        let samples: Vec<f32> = (0..BLOCK as u64 * CHANNELS as u64)
            .map(|i| {
                let k = self.sample_time * CHANNELS as u64 + i;
                let x = (k.wrapping_mul(2_654_435_761) % 2001) as f32 / 1000.0 - 1.0;
                x * if i % 2 == 0 { 0.5 } else { 0.9 }
            })
            .collect();
        self.sample_time += u64::from(BLOCK);
        let s = self.session.as_mut().unwrap();
        s.block(&b, &samples);
        // The writer publishes after each drain.
        if self.sample_time.is_multiple_of(u64::from(BLOCK) * 3) {
            s.flush_live();
        }
    }

    fn play(&mut self, from: f64, blocks: usize) {
        for i in 0..blocks {
            self.block(from + (i as u64 * u64::from(BLOCK)) as f64 * BPS);
        }
    }

    fn poll(&mut self) {
        self.live.lock().drain(&mut self.chunks, &mut self.notes);
    }

    fn finish(&mut self) -> RecordedTakes {
        let takes = self.session.take().unwrap().finish().unwrap();
        self.poll();
        takes
    }

    fn wav(&self, file: &str) -> (usize, Vec<f32>) {
        let bytes =
            std::fs::read(self.tmp.path().join(self.project.to_string()).join(file)).unwrap();
        let channels = usize::from(u16::from_le_bytes([bytes[22], bytes[23]]));
        let samples = bytes[44..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        (channels, samples)
    }

    /// Live peaks of (track, take), reassembled from the chunks by `first_peak`.
    fn live_peaks(&self, track: TrackId, take: u32) -> (f64, Vec<f32>, Vec<f32>) {
        let mut start = None;
        let (mut min, mut max) = (Vec::new(), Vec::new());
        for c in self
            .chunks
            .iter()
            .filter(|c| c.track == track && c.take == take)
        {
            assert_eq!(c.frames_per_peak, FRAMES_PER_PEAK);
            assert_eq!(c.sample_rate, SR);
            assert_eq!(c.first_peak, min.len() as u64, "chunks are contiguous");
            assert_eq!(
                *start.get_or_insert(c.start),
                c.start,
                "same start in every chunk"
            );
            min.extend(&c.min);
            max.extend(&c.max);
        }
        (start.expect("live peaks for the take"), min, max)
    }
}

/// Min/max peaks of an interleaved WAV at `FRAMES_PER_PEAK`, all channels merged.
fn wav_peaks(channels: usize, samples: &[f32]) -> (Vec<f32>, Vec<f32>) {
    samples
        .chunks(channels * FRAMES_PER_PEAK as usize)
        .map(|w| {
            w.iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), s| {
                    (lo.min(*s), hi.max(*s))
                })
        })
        .unzip()
}

fn assert_take_matches(rig: &Rig, takes: &RecordedTakes, take_no: u32, index: usize) {
    let take = &takes.audio[index];
    let (start, min, max) = rig.live_peaks(take.track, take_no);
    assert_eq!(start, take.start, "live start is the committed take start");
    let (channels, samples) = rig.wav(&take.file);
    assert_eq!(channels, usize::from(take.channels));
    let (wmin, wmax) = wav_peaks(channels, &samples);
    assert_eq!(min, wmin, "live min peaks match the WAV");
    assert_eq!(max, wmax, "live max peaks match the WAV");
    assert_eq!(
        min.len() as u64,
        take.frames.div_ceil(u64::from(FRAMES_PER_PEAK))
    );
}

#[test]
fn live_peaks_match_the_written_takes_and_skip_the_count_in() {
    let stereo = target(1, 0, 2);
    let mono = target(2, 1, 1);
    // Count-in: one beat before the record position (keep from 0), 1000 samples latency.
    let mut rig = Rig::new(
        "live-countin",
        vec![stereo.clone(), mono.clone()],
        (0.0, None),
        1000,
    );
    rig.play(-1.0, 200);
    // Polled mid-take: only complete peaks so far, already covering most of it.
    rig.poll();
    assert!(!rig.chunks.is_empty(), "peaks arrive while recording");
    let takes = rig.finish();
    assert_eq!(takes.audio.len(), 2);
    for (i, t) in takes.audio.iter().enumerate() {
        assert!(t.start.abs() < 1e-9, "starts at the record position");
        assert_take_matches(&rig, &takes, 1, i);
    }
    // Nothing of the count-in: the take is exactly what was kept.
    let played = 200 * u64::from(BLOCK);
    let count_in = SR as u64 / 2;
    assert_eq!(takes.audio[0].frames, played - count_in - 1000);
}

#[test]
fn live_peaks_cover_only_the_punch_range() {
    let mut rig = Rig::new("live-punch", vec![target(1, 0, 2)], (1.0, Some(2.0)), 300);
    rig.play(0.0, 400);
    let takes = rig.finish();
    assert_eq!(takes.audio.len(), 1);
    let t = &takes.audio[0];
    assert!((t.start - 1.0).abs() < BPS, "{}", t.start);
    assert!(((t.frames as f64 * BPS) - 1.0).abs() < 2.0 * BPS);
    assert_take_matches(&rig, &takes, 1, 0);
}

#[test]
fn each_loop_wrap_starts_a_new_live_take() {
    let mut rig = Rig::new("live-loop", vec![target(1, 0, 1)], (0.0, None), 512);
    // Loop [0, 2): 2 beats = 24000 frames (not a multiple of the block: wrap mid-block is
    // modelled as a new block at the loop start).
    let loop_blocks = 24_000 / BLOCK as usize;
    for _ in 0..3 {
        rig.play(0.0, loop_blocks);
    }
    let takes = rig.finish();
    assert_eq!(takes.audio.len(), 3, "{takes:?}");
    for i in 0..takes.audio.len() {
        assert_take_matches(&rig, &takes, i as u32 + 1, i);
    }
    let numbers: std::collections::BTreeSet<u32> = rig.chunks.iter().map(|c| c.take).collect();
    assert_eq!(numbers.into_iter().collect::<Vec<_>>(), vec![1, 2, 3]);
}

#[test]
fn live_notes_start_then_end_like_the_committed_ones() {
    let mut rig = Rig::new("live-midi", vec![], (1.0, None), 0);
    let midi = |position: f64, data: [u8; 3]| EngineMidi {
        sample_time: 0,
        position,
        beats_per_sample: BPS,
        data,
    };
    let s = rig.session.as_mut().unwrap();
    s.midi(midi(0.5, [0x90, 60, 100])); // count-in: dropped
    s.midi(midi(1.5, [0x90, 64, 90]));
    s.midi(midi(1.75, [0x90, 67, 80]));
    rig.poll();
    assert_eq!(rig.notes.len(), 2);
    assert_eq!(
        (rig.notes[0].pitch, rig.notes[0].start, rig.notes[0].length),
        (64, 1.5, None)
    );
    assert_eq!(rig.notes[1].velocity, 80);
    let s = rig.session.as_mut().unwrap();
    s.midi(midi(2.5, [0x80, 64, 0]));
    s.midi(midi(2.6, [0x90, 60, 0])); // note-off of a count-in note: ignored
    rig.poll();
    assert_eq!(rig.notes.len(), 3);
    let end = &rig.notes[2];
    assert_eq!((end.pitch, end.start, end.length), (64, 1.5, Some(1.0)));
    assert_eq!(end.velocity, 90);
    assert_eq!(end.track, TrackId::NIL, "the controller assigns the tracks");
    rig.finish();
}

#[test]
fn a_stalled_controller_drops_the_oldest_chunks() {
    let mut rig = Rig::new("live-stall", vec![target(1, 0, 1)], (0.0, None), 0);
    // Each block completes one peak; flush after every block.
    for i in 0..(MAX_CHUNKS + 10) {
        rig.block(i as f64 * f64::from(BLOCK) * BPS);
        rig.session.as_mut().unwrap().flush_live();
    }
    assert_eq!(rig.live.lock().dropped(), 10);
    rig.poll();
    assert_eq!(rig.chunks.len(), MAX_CHUNKS);
    assert_eq!(rig.chunks[0].first_peak, 10, "the oldest were dropped");
    rig.finish();
}
