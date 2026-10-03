//! `fx-analysis` end to end on the engine side: the real SpectrumAnalyzer and Tuner nodes
//! on a monitored audio track publish frames through the engine's analysis channel
//! (`EngineHandle::{watch_analysis, poll_analysis}`, CONTRACTS.md §12.4.3) at ~30 Hz, with
//! the encodings the controller decodes.

use ether_core::analysis::{ANALYSIS_HZ, AnalysisKind};
use ether_core::graph::{ChainEntry, RenderGraphDesc, TrackDesc};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, TrackId, TrackKind, Ulid};
use ether_core::{EngineConfig, create};
use ether_devices::NoSamples;

const SR: u32 = 48_000;
const BLOCK: usize = 256;

fn track(id: u128, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
    TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id: TrackId(Ulid(id)),
        kind,
        chain: vec![],
        output,
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        monitor: false,
        armed: false,
        clips: vec![],
        automation: vec![],
        racks: Vec::new(),
        expression: Default::default(),
        hw_io: Vec::new(),
    }
}

#[test]
fn spectrum_and_tuner_frames_flow_through_the_engine() {
    let mut parts = create(EngineConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_nodes: 64,
        max_events_per_block: 256,
        ..EngineConfig::default()
    });
    let node = |ty| ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    let spectrum = parts
        .handle
        .add_node(node(BuiltinDeviceType::SpectrumAnalyzer))
        .unwrap();
    let tuner = parts
        .handle
        .add_node(node(BuiltinDeviceType::Tuner))
        .unwrap();
    let master = track(1, TrackKind::Master, None);
    let mut audio = track(2, TrackKind::Audio, Some(master.id));
    audio.audio_input = Some((0, 2));
    audio.monitor = true;
    audio.chain = [spectrum, tuner]
        .into_iter()
        .map(|node| ChainEntry {
            node,
            enabled: true,
            sidechain: None,
        })
        .collect();
    parts
        .handle
        .publish(RenderGraphDesc {
            tracks: vec![master, audio],
            ..Default::default()
        })
        .unwrap();
    parts.handle.watch_analysis(spectrum, true).unwrap();
    parts.handle.watch_analysis(tuner, true).unwrap();

    // One second of A4 at -6 dBFS on both inputs.
    let blocks = SR as usize / BLOCK;
    let mut out = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    let mut input = vec![0.0f32; BLOCK];
    let mut frames = Vec::new();
    for b in 0..blocks {
        for (i, s) in input.iter_mut().enumerate() {
            let t = (b * BLOCK + i) as f32 / SR as f32;
            *s = 0.5 * (std::f32::consts::TAU * 440.0 * t).sin();
        }
        let ins: [&[f32]; 2] = [&input, &input];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        parts.engine.process(&ins, &mut outs, BLOCK);
        parts.handle.poll_analysis(|f| frames.push(*f));
    }
    let of = |node, kind| {
        frames
            .iter()
            .filter(|f| f.node == node && f.kind == kind)
            .collect::<Vec<_>>()
    };
    let spectra = of(spectrum, AnalysisKind::Spectrum);
    let tunings = of(tuner, AnalysisKind::Tuner);
    for n in [spectra.len(), tunings.len()] {
        assert!(
            (ANALYSIS_HZ as usize - 4..=ANALYSIS_HZ as usize + 1).contains(&n),
            "{n} frames"
        );
    }
    // Spectrum: [min_hz, max_hz, 256 bins], loudest bin at 440 Hz.
    let s = spectra.last().unwrap().values();
    assert_eq!(s.len(), 258);
    let (i, _) = s[2..].iter().enumerate().fold(
        (0, f32::MIN),
        |b, (i, v)| if *v > b.1 { (i, *v) } else { b },
    );
    let hz = s[0] * (s[1] / s[0]).powf(i as f32 / 255.0);
    assert!((hz / 440.0 - 1.0).abs() < 0.03, "{hz}");
    // Tuner: [hz, note, cents, confidence, level_db] = A4, in tune, ~-9 dBFS RMS.
    let t = tunings.last().unwrap().values();
    assert_eq!(t.len(), 5);
    assert!((t[0] - 440.0).abs() < 1.0, "{t:?}");
    assert_eq!(t[1], 69.0);
    assert!(t[2].abs() < 3.0, "{t:?}");
    assert!(t[3] > 0.9, "{t:?}");
    assert!((t[4] + 9.03).abs() < 0.5, "{t:?}");
    // Unwatched: no more frames.
    parts.handle.watch_analysis(spectrum, false).unwrap();
    parts.handle.watch_analysis(tuner, false).unwrap();
    for _ in 0..blocks / 2 {
        let ins: [&[f32]; 2] = [&input, &input];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        parts.engine.process(&ins, &mut outs, BLOCK);
    }
    // (frames already in flight when the unwatch landed may still arrive)
    let mut late = 0;
    parts.handle.poll_analysis(|_| late += 1);
    assert!(late <= 2, "{late}");
}
