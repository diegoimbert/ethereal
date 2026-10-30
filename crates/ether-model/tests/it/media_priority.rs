//! `media_priority`: which media load first (see the module docs).

use std::collections::{BTreeMap, HashMap};

use ether_model::media_priority::{Listening, MediaTier, file_order, media_priority};
use ether_model::*;

struct Fx {
    p: Project,
    ids: IdGen,
    now: u64,
    order: Option<OrderKey>,
}

impl Fx {
    fn new() -> Self {
        let mut ids = IdGen::new(7);
        let p = Project::new(&mut ids, 1_000);
        Self {
            p,
            ids,
            now: 1_000,
            order: None,
        }
    }
    fn id<I: Id>(&mut self) -> I {
        self.now += 1;
        self.ids.next(self.now)
    }
    fn next_order(&mut self) -> OrderKey {
        let k = OrderKey::between(self.order.as_ref(), None);
        self.order = Some(k.clone());
        k
    }
    fn track(&mut self, kind: TrackKind) -> TrackId {
        let order = self.next_order();
        let t = Track {
            vca: None,
            freeze: None,
            id: self.id(),
            kind,
            name: "t".into(),
            color: Color(0),
            order,
            parent: None,
            mixer: TrackMixer::default(),
            input: TrackInput::None,
            output: TrackOutput::Default,
            monitor: MonitorMode::Auto,
            scale: Default::default(),
            mpe: None,
        };
        let id = t.id;
        self.p.tracks.insert(id, t);
        id
    }
    /// A media of `seconds` stereo 48 kHz.
    fn media(&mut self, name: &str, seconds: f64) -> MediaId {
        let id = self.id();
        self.p.media.insert(
            id,
            MediaRef {
                id,
                name: name.into(),
                file: format!("media/{name}.wav"),
                sample_rate: 48_000,
                channels: 2,
                frames: (seconds * 48_000.0) as u64,
                hash: None,
                location: MediaLocation::Project,
            },
        );
        id
    }
    fn clip(&mut self, track: TrackId, start: f64, length: f64, content: ClipContent) -> ClipId {
        let id = self.id();
        self.p.clips.insert(
            id,
            Clip {
                lane: None,
                id,
                track,
                start: Beats(start),
                name: String::new(),
                color: None,
                muted: false,
                length: Beats(length),
                offset: Beats::ZERO,
                looping: ClipLoop {
                    enabled: false,
                    start: Beats::ZERO,
                    end: Beats(length),
                },
                content,
            },
        );
        id
    }
    fn audio(&mut self, track: TrackId, media: MediaId, start: f64, length: f64) -> ClipId {
        self.clip(
            track,
            start,
            length,
            ClipContent::Audio(AudioContent {
                media,
                gain: Decibels(0.0),
                transpose: 0.0,
                fade_in: Beats(0.0),
                fade_out: Beats(0.0),
                warp: WarpSettings::default(),
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                reversed: false,
            }),
        )
    }
    fn note(&mut self, clip: ClipId, start: f64, pitch: u8, velocity: f32) {
        let id = self.id();
        self.p.notes.insert(
            id,
            Note {
                id,
                clip,
                pitch,
                velocity,
                release_velocity: 0.5,
                start: Beats(start),
                duration: Beats(0.5),
                muted: false,
            },
        );
    }
    fn device(&mut self, track: TrackId, device: BuiltinDevice) -> DeviceId {
        let order = self.next_order();
        let id = self.id();
        self.p.devices.insert(
            id,
            Device {
                id,
                track,
                order,
                name: "d".into(),
                enabled: true,
                kind: DeviceKind::Builtin { device },
                params: BTreeMap::new(),
                sidechain: None,
                sidechain_tap: None,
                pad: None,
                chain: None,
                midi_sidechain: None,
            },
        );
        id
    }
    fn pad(&mut self, rack: DeviceId, track: TrackId, note: u8, sample: MediaId) {
        let pad = DrumPad {
            id: self.id(),
            rack,
            note,
            name: String::new(),
            color: None,
            choke_group: None,
            volume: Decibels(0.0),
            pan: Pan(0.0),
            mute: false,
        };
        let pid = pad.id;
        self.p.drum_pads.insert(pid, pad);
        let d = self.device(track, sampler(sample));
        self.p.devices.get_mut(&d).unwrap().pad = Some(pid);
    }
    fn order(&mut self, at: f64) -> Vec<String> {
        self.p.invalidate_index();
        let l = Listening {
            at: Beats(at),
            loop_region: None,
        };
        file_order(&self.p, &l, &HashMap::new())
    }
    fn tier(&self, media: MediaId) -> MediaTier {
        let l = Listening::of(&self.p, Beats(0.0));
        media_priority(&self.p, &l, &HashMap::new())
            .into_iter()
            .find(|m| m.media == media)
            .unwrap()
            .tier
    }
}

fn sampler(m: MediaId) -> BuiltinDevice {
    BuiltinDevice::Sampler {
        sample: Some(m),
        slices: SliceSettings::default(),
    }
}

fn file(name: &str) -> String {
    format!("media/{name}.wav")
}

fn zone(m: MediaId, keys: (u8, u8), vel: (u8, u8)) -> SampleZone {
    SampleZone {
        media: Some(m),
        keys: Zone {
            lo: keys.0,
            hi: keys.1,
        },
        velocities: Zone {
            lo: vel.0,
            hi: vel.1,
        },
        ..SampleZone::default()
    }
}

#[test]
fn a_long_single_use_sample_beats_several_short_one_shots() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio);
    let pad = f.media("pad", 120.0);
    let hit = f.media("hit", 0.5);
    f.audio(t, pad, 0.0, 256.0);
    let u = f.track(TrackKind::Audio);
    for i in 0..8 {
        f.audio(u, hit, i as f64 * 4.0, 1.0);
    }
    assert_eq!(f.order(0.0), vec![file("pad"), file("hit")]);
}

#[test]
fn what_plays_next_comes_first_and_the_playhead_moves_it() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio);
    let intro = f.media("intro", 8.0);
    let drop = f.media("drop", 8.0);
    f.audio(t, intro, 0.0, 16.0);
    f.audio(t, drop, 128.0, 16.0);
    assert_eq!(f.order(0.0), vec![file("intro"), file("drop")]);
    assert_eq!(f.order(120.0), vec![file("drop"), file("intro")]);
    // Material behind the playhead comes after the end of the song.
    assert_eq!(f.order(140.0), vec![file("drop"), file("intro")]);
}

#[test]
fn the_transport_loop_plays_first() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio);
    let early = f.media("early", 8.0);
    let looped = f.media("looped", 8.0);
    f.audio(t, early, 0.0, 16.0);
    f.audio(t, looped, 64.0, 16.0);
    f.p.settings.loop_enabled = true;
    f.p.settings.loop_region = BeatRange {
        start: Beats(64.0),
        end: Beats(80.0),
    };
    let l = Listening::of(&f.p, Beats(70.0));
    assert_eq!(
        file_order(&f.p, &l, &HashMap::new()),
        vec![file("looped"), file("early")]
    );
}

#[test]
fn muted_frozen_and_disabled_uses_go_after_what_is_heard() {
    let mut f = Fx::new();
    let heard = f.track(TrackKind::Audio);
    let muted = f.track(TrackKind::Audio);
    let short = f.media("short", 1.0);
    let long_muted = f.media("long-muted", 120.0);
    f.audio(heard, short, 200.0, 1.0);
    f.audio(muted, long_muted, 0.0, 256.0);
    f.p.tracks.get_mut(&muted).unwrap().mixer.mute = true;
    // A muted clip, and a sampler that is switched off.
    let mclip = f.media("muted-clip", 4.0);
    let c = f.audio(heard, mclip, 0.0, 8.0);
    f.p.clips.get_mut(&c).unwrap().muted = true;
    let midi = f.track(TrackKind::Midi);
    let off = f.media("off", 1.0);
    let d = f.device(midi, sampler(off));
    f.p.devices.get_mut(&d).unwrap().enabled = false;
    let mc = f.clip(midi, 0.0, 4.0, ClipContent::Midi);
    f.note(mc, 0.0, 60, 0.8);
    // A frozen track: its render is heard, its clips are not.
    let frozen = f.track(TrackKind::Audio);
    let raw = f.media("raw", 4.0);
    let render = f.media("render", 8.0);
    f.audio(frozen, raw, 0.0, 8.0);
    f.p.tracks.get_mut(&frozen).unwrap().freeze = Some(TrackFreeze {
        media: render,
        start: Seconds(0.0),
        up_to: None,
    });
    assert_eq!(f.tier(short), MediaTier::Heard);
    assert_eq!(f.tier(render), MediaTier::Heard);
    for m in [long_muted, mclip, off, raw] {
        assert_eq!(f.tier(m), MediaTier::Silent);
    }
    let order = f.order(0.0);
    assert_eq!(&order[..2], &[file("render"), file("short")]);
}

#[test]
fn multisampler_zones_no_note_triggers_come_last() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let low = f.media("low", 2.0);
    let mid_soft = f.media("mid-soft", 2.0);
    let mid_hard = f.media("mid-hard", 2.0);
    let high = f.media("high", 2.0);
    let other = f.track(TrackKind::Audio);
    let loop_ = f.media("loop", 30.0);
    f.audio(other, loop_, 64.0, 4.0);
    f.device(
        t,
        BuiltinDevice::MultiSampler {
            zones: vec![
                zone(low, (0, 47), (1, 127)),
                zone(mid_soft, (48, 59), (1, 63)),
                zone(mid_hard, (48, 59), (64, 127)),
                zone(high, (60, 127), (1, 127)),
            ],
        },
    );
    let c = f.clip(t, 0.0, 16.0, ClipContent::Midi);
    for i in 0..16 {
        f.note(c, i as f64, 50, 0.9);
    }
    assert_eq!(f.tier(mid_hard), MediaTier::Heard);
    assert_eq!(f.tier(loop_), MediaTier::Heard);
    for m in [low, mid_soft, high] {
        assert_eq!(f.tier(m), MediaTier::Idle);
    }
    let order = f.order(0.0);
    assert_eq!(&order[..2], &[file("mid-hard"), file("loop")]);
    // The unused zones are all there, last, smallest first (all the same size: by name).
    assert_eq!(&order[2..], &[file("high"), file("low"), file("mid-soft")]);
}

#[test]
fn a_note_changing_effect_makes_every_zone_count() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let a = f.media("a", 2.0);
    let b = f.media("b", 2.0);
    f.device(t, BuiltinDevice::Arpeggiator);
    f.device(
        t,
        BuiltinDevice::MultiSampler {
            zones: vec![zone(a, (0, 59), (1, 127)), zone(b, (60, 127), (1, 127))],
        },
    );
    let c = f.clip(t, 0.0, 4.0, ClipContent::Midi);
    f.note(c, 0.0, 40, 0.5);
    assert_eq!(f.tier(a), MediaTier::Heard);
    assert_eq!(f.tier(b), MediaTier::Heard);
}

#[test]
fn drum_pads_rank_by_how_often_they_are_hit() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let kick = f.media("kick", 0.5);
    let snare = f.media("snare", 0.5);
    let crash = f.media("crash", 0.5);
    let rack = f.device(t, BuiltinDevice::DrumRack { macros: None });
    f.pad(rack, t, 36, kick);
    f.pad(rack, t, 38, snare);
    f.pad(rack, t, 49, crash);
    let c = f.clip(t, 0.0, 16.0, ClipContent::Midi);
    for i in 0..16 {
        f.note(c, i as f64, 36, 1.0);
        if i % 4 == 1 {
            f.note(c, i as f64, 38, 1.0);
        }
    }
    assert_eq!(f.tier(crash), MediaTier::Idle);
    assert_eq!(
        f.order(0.0),
        vec![file("kick"), file("snare"), file("crash")]
    );
}

#[test]
fn a_looping_midi_clip_counts_every_pass() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Midi);
    let looped = f.media("looped", 0.5);
    f.device(t, sampler(looped));
    let c = f.clip(t, 0.0, 64.0, ClipContent::Midi);
    f.p.clips.get_mut(&c).unwrap().looping = ClipLoop {
        enabled: true,
        start: Beats(0.0),
        end: Beats(4.0),
    };
    f.note(c, 0.0, 60, 1.0);
    let u = f.track(TrackKind::Audio);
    let once = f.media("once", 0.5);
    f.audio(u, once, 0.0, 1.0);
    let l = Listening::of(&f.p, Beats(0.0));
    let r = media_priority(&f.p, &l, &HashMap::new());
    let looped_r = r.iter().find(|m| m.media == looped).unwrap();
    assert_eq!(looped_r.heard, 16.0 * 0.5, "16 passes of a half-beat note");
    assert_eq!(r[0].media, looped);
}

#[test]
fn ties_go_to_the_smaller_file() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio);
    let u = f.track(TrackKind::Audio);
    let big = f.media("big", 60.0);
    let small = f.media("small", 4.0);
    f.audio(t, big, 0.0, 16.0);
    f.audio(u, small, 0.0, 15.0);
    assert_eq!(f.order(0.0), vec![file("small"), file("big")]);
    // Given sizes win over the estimate.
    let sizes = HashMap::from([(file("big"), 10), (file("small"), 1 << 20)]);
    let l = Listening::of(&f.p, Beats(0.0));
    assert_eq!(
        file_order(&f.p, &l, &sizes),
        vec![file("big"), file("small")]
    );
}

#[test]
fn unused_media_are_listed_last() {
    let mut f = Fx::new();
    let t = f.track(TrackKind::Audio);
    let used = f.media("used", 1.0);
    let unused = f.media("unused", 1.0);
    f.audio(t, used, 0.0, 4.0);
    assert_eq!(f.tier(unused), MediaTier::Idle);
    assert_eq!(f.order(0.0), vec![file("used"), file("unused")]);
}

// ─── Time to first playable (a synthetic big project) ────────────────────────────────────

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// 128 bars (512 beats): 10 stems (90-180 s, entering at different bars), 10 two-bar loops
/// used by many clips, 40 one-shot effects, 4 drum racks of 16 pads (5 played), 2
/// multisamplers of 48 zones (16 keys × 3 velocity layers; a few keys played, middle layer),
/// a muted track with a long take. 231 media, imported in a shuffled order.
fn big_project() -> (Fx, Vec<MediaId>) {
    let mut f = Fx::new();
    let mut rng = Rng(42);
    let mut imported = Vec::new();
    let song = 512.0;
    for i in 0..10 {
        let t = f.track(TrackKind::Audio);
        let m = f.media(&format!("stem-{i}"), 90.0 + rng.below(90) as f64);
        imported.push(m);
        let start = 32.0 * rng.below(8) as f64;
        f.audio(t, m, start, song - start);
    }
    for i in 0..10 {
        let t = f.track(TrackKind::Audio);
        let m = f.media(&format!("loop-{i}"), 4.0);
        imported.push(m);
        let section = 64.0 * rng.below(6) as f64;
        for k in 0..(8 + rng.below(8)) {
            f.audio(t, m, section + 8.0 * k as f64, 8.0);
        }
    }
    for i in 0..10 {
        let t = f.track(TrackKind::Audio);
        for k in 0..4 {
            let m = f.media(&format!("fx-{i}-{k}"), 1.0 + rng.below(3) as f64);
            imported.push(m);
            f.audio(t, m, 4.0 * rng.below(128) as f64, 2.0);
        }
    }
    for r in 0..4 {
        let t = f.track(TrackKind::Midi);
        let rack = f.device(t, BuiltinDevice::DrumRack { macros: None });
        for pad in 0..16u8 {
            let m = f.media(&format!("drum-{r}-{pad}"), 0.3 + 0.05 * pad as f64);
            imported.push(m);
            f.pad(rack, t, 36 + pad, m);
        }
        let section = 64.0 * rng.below(4) as f64;
        for bar in 0..32 {
            let c = f.clip(t, section + 4.0 * bar as f64, 4.0, ClipContent::Midi);
            for beat in 0..4 {
                f.note(c, beat as f64, 36, 1.0);
                f.note(c, beat as f64 + 0.5, 42, 0.7);
                if beat % 2 == 1 {
                    f.note(c, beat as f64, 38, 0.9);
                }
            }
            if bar % 4 == 0 {
                f.note(c, 0.0, 49, 1.0);
                f.note(c, 3.5, 46, 0.6);
            }
        }
    }
    for s in 0..2 {
        let t = f.track(TrackKind::Midi);
        let mut zones = Vec::new();
        for k in 0..16u8 {
            for (layer, vel) in [(1u8, 42u8), (43, 85), (86, 127)].into_iter().enumerate() {
                let m = f.media(&format!("keys-{s}-{k}-{layer}"), 3.0 + rng.below(5) as f64);
                imported.push(m);
                zones.push(zone(m, (36 + 3 * k, 38 + 3 * k), vel));
            }
        }
        f.device(t, BuiltinDevice::MultiSampler { zones });
        let section = 128.0 * rng.below(3) as f64;
        for bar in 0..16 {
            let c = f.clip(t, section + 8.0 * bar as f64, 8.0, ClipContent::Midi);
            for (i, key) in [48u8, 52, 55, 60].into_iter().enumerate() {
                f.note(c, i as f64 * 2.0, key, 0.5);
            }
        }
    }
    let t = f.track(TrackKind::Audio);
    let m = f.media("muted-take", 150.0);
    imported.push(m);
    f.audio(t, m, 0.0, song);
    f.p.tracks.get_mut(&t).unwrap().mixer.mute = true;
    // Shuffled import order (what a host pushes first, the relay caches in that order).
    for i in (1..imported.len()).rev() {
        imported.swap(i, rng.below(i as u64 + 1) as usize);
    }
    (f, imported)
}

/// The shared copy's order before media_priority: the audio clips by start, frozen tracks,
/// then the devices' samples by file.
fn arrangement_order(p: &Project) -> Vec<MediaId> {
    let mut first: BTreeMap<MediaId, (u8, f64)> = BTreeMap::new();
    for c in p.clips.values() {
        if let ClipContent::Audio(a) = &c.content {
            let e = first.entry(a.media).or_insert((0, c.start.0));
            e.1 = e.1.min(c.start.0);
        }
    }
    let mut ids: Vec<MediaId> = p.media.keys().copied().collect();
    ids.sort_by(|a, b| {
        let ka = first.get(a).copied().unwrap_or((2, 0.0));
        let kb = first.get(b).copied().unwrap_or((2, 0.0));
        ka.0.cmp(&kb.0)
            .then(ka.1.total_cmp(&kb.1))
            .then_with(|| p.media[a].file.cmp(&p.media[b].file))
    });
    ids
}

/// Seconds until each file is in, downloaded one after the other at `bytes_per_s`.
fn arrivals(p: &Project, order: &[MediaId], bytes_per_s: f64) -> HashMap<MediaId, f64> {
    let mut t = 0.0;
    order
        .iter()
        .map(|m| {
            let r = &p.media[m];
            t += (r.frames * u64::from(r.channels) * 2) as f64 / bytes_per_s;
            (*m, t)
        })
        .collect()
}

/// What a download order gives, from `at`: seconds until half of the media that play in
/// the next 8 bars are in, until all of them are, until everything heard is, and the least
/// possible for all of the next 8 bars (their bytes alone).
fn playable(p: &Project, order: &[MediaId], at: f64) -> [f64; 4] {
    let rate = 8.0 * 1024.0 * 1024.0;
    let got = arrivals(p, order, rate);
    let ranks = media_priority(p, &Listening::of(p, Beats(at)), &HashMap::new());
    let heard: Vec<_> = ranks
        .iter()
        .filter(|m| m.tier == MediaTier::Heard)
        .collect();
    let mut soon: Vec<f64> = heard
        .iter()
        .filter(|m| m.soonest.is_some_and(|s| s < 32.0))
        .map(|m| got[&m.media])
        .collect();
    soon.sort_by(f64::total_cmp);
    let least: f64 = heard
        .iter()
        .filter(|m| m.soonest.is_some_and(|s| s < 32.0))
        .map(|m| m.size as f64 / rate)
        .sum();
    let all = heard.iter().map(|m| got[&m.media]).fold(0.0, f64::max);
    [soon[soon.len() / 2], *soon.last().unwrap(), all, least]
}

/// Modeled at a data channel's ~8 MiB/s, one file after the other: `cargo test -p
/// ether-model --test it -- --nocapture media_priority::time_to_first_playable`.
#[test]
fn time_to_first_playable() {
    let (f, imported) = big_project();
    let p = &f.p;
    let total: u64 = p
        .media
        .values()
        .map(|m| m.frames * u64::from(m.channels) * 2)
        .sum();
    let t0 = std::time::Instant::now();
    let ranks = media_priority(p, &Listening::of(p, Beats(0.0)), &HashMap::new());
    let took = t0.elapsed();
    let prio = |at: f64| -> Vec<MediaId> {
        media_priority(p, &Listening::of(p, Beats(at)), &HashMap::new())
            .into_iter()
            .map(|m| m.media)
            .collect()
    };
    println!(
        "{} media, {:.0} MiB, {} notes; ranking took {took:?}",
        p.media.len(),
        total as f64 / (1 << 20) as f64,
        p.notes.len()
    );
    let idle = ranks.iter().filter(|m| m.tier == MediaTier::Idle).count();
    println!("idle (no note triggers them): {idle}");
    let arrangement = arrangement_order(p);
    for at in [0.0, 256.0] {
        let rows = [
            ("relay cache order (jam, before)", imported.clone()),
            ("arrangement order (copy, before)", arrangement.clone()),
            ("media_priority (after)", prio(at)),
        ];
        let mut results = Vec::new();
        for (name, order) in &rows {
            let [half, soon, all, least] = playable(p, order, at);
            println!(
                "from beat {at:>3}: {name:<34} next 8 bars: half {half:>5.1} s, all {soon:>5.1} s \
                 (least {least:>4.1} s); everything heard {all:>5.1} s"
            );
            results.push((soon, least));
        }
        let (after, before) = (results[2], [results[0], results[1]]);
        for b in before {
            assert!(after.0 <= b.0, "{results:?}");
            // From the start, at least twice as fast as before.
            assert!(at > 0.0 || after.0 * 2.0 <= b.0, "{results:?}");
        }
        // What plays next is in about as soon as its bytes alone allow (generous bound).
        assert!(after.0 <= after.1 * 1.1 + 0.5, "{results:?}");
    }
}

/// A crafted project (a shared copy or a session ranks someone else's project): many notes
/// in a tiny content loop, heard by a sampler with many zones. The ranking stays bounded.
#[test]
fn crafted_loops_and_zones_stay_cheap() {
    let mut f = Fx::new();
    let media: Vec<MediaId> = (0..512).map(|i| f.media(&format!("z{i}"), 1.0)).collect();
    let zones = media.iter().map(|m| zone(*m, (0, 127), (1, 127))).collect();
    let t = f.track(TrackKind::Midi);
    f.device(t, BuiltinDevice::MultiSampler { zones });
    let c = f.clip(t, 0.0, 100_000.0, ClipContent::Midi);
    let clip = f.p.clips.get_mut(&c).unwrap();
    clip.looping = ClipLoop {
        enabled: true,
        start: Beats::ZERO,
        end: Beats(1.0),
    };
    for i in 0..20_000 {
        f.note(c, i as f64 / 20_000.0, 60, 0.9);
    }
    f.p.invalidate_index();
    let t0 = std::time::Instant::now();
    let ranks = media_priority(&f.p, &Listening::of(&f.p, Beats(0.0)), &HashMap::new());
    let took = t0.elapsed();
    println!("crafted: {} media ranked in {took:?}", ranks.len());
    assert_eq!(ranks.len(), 512);
    assert!(ranks.iter().all(|r| r.tier == MediaTier::Heard));
    // ~0.1 s here; one hit per note, loop pass and zone took minutes.
    assert!(took.as_secs() < 20, "ranking took {took:?}");
}
