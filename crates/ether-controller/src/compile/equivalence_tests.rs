//! The one-pass compiler produces exactly the graph of the scanning one ([`super::reference`],
//! run on an un-indexed copy so every lookup is a table scan) on random projects: groups,
//! returns, sends, devices with and without engine nodes, MIDI and audio clips, muted notes
//! and start/pitch ties, track and clip automation (disabled and empty lanes, unresolvable
//! targets, point time ties), warp markers, take lanes and comps, swing, and expressions.

use ether_core::NodeKey;
use ether_core::protocol::model::*;

use super::{CompileContext, builtin_descriptors, compile_graph_with};

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
        (!items.is_empty()).then(|| items[self.below(items.len())])
    }
    /// Coarse beats (many ties) or arbitrary ones.
    fn beats(&mut self, max: f64) -> Beats {
        if self.chance(50) {
            Beats((self.below((max * 4.0) as usize) as f64) / 4.0)
        } else {
            Beats((self.next() >> 11) as f64 / (1u64 << 53) as f64 * max)
        }
    }
}

fn insert(p: &mut Project, entity: Entity) {
    let _ = p.apply(&Op::Insert { entity });
}

struct Gen {
    p: Project,
    ids: IdGen,
    r: Rng,
    now: u64,
}

impl Gen {
    fn id<I: Id>(&mut self) -> I {
        self.now += 1;
        self.ids.next(self.now)
    }

    fn keys<K: Copy, V>(m: &std::collections::BTreeMap<K, V>) -> Vec<K> {
        m.keys().copied().collect()
    }

    fn tracks(&self, kinds: &[TrackKind]) -> Vec<TrackId> {
        self.p
            .tracks
            .values()
            .filter(|t| kinds.contains(&t.kind))
            .map(|t| t.id)
            .collect()
    }

    fn track(&mut self) {
        let kind = [
            TrackKind::Midi,
            TrackKind::Midi,
            TrackKind::Audio,
            TrackKind::Group,
            TrackKind::Return,
        ][self.r.below(5)];
        let groups = self.tracks(&[TrackKind::Group]);
        let parent = if self.r.chance(40) {
            self.r.pick(&groups)
        } else {
            None
        };
        let orders = OrderKey::n_between(None, None, 8);
        let order = orders[self.r.below(orders.len())].clone();
        let id = self.id();
        insert(
            &mut self.p,
            Entity::Track(Track {
                vca: Default::default(),
                freeze: None,
                id,
                kind,
                name: "T".into(),
                color: Color(0x123456),
                order,
                parent,
                mixer: TrackMixer {
                    volume: Decibels(-(self.r.below(20) as f32)),
                    pan: Pan(0.25),
                    mute: self.r.chance(10),
                    solo: self.r.chance(10),
                },
                input: TrackInput::None,
                output: TrackOutput::Default,
                monitor: [MonitorMode::Auto, MonitorMode::In, MonitorMode::Off][self.r.below(3)],
                scale: Default::default(),
                mpe: None,
            }),
        );
    }

    fn device(&mut self) {
        let Some(track) = self.r.pick(&Self::keys(&self.p.tracks)) else {
            return;
        };
        let orders = OrderKey::n_between(None, None, 4);
        let order = orders[self.r.below(orders.len())].clone();
        let id = self.id();
        insert(
            &mut self.p,
            Entity::Device(Device {
                sidechain_tap: None,
                chain: None,
                id,
                track,
                order,
                name: "D".into(),
                enabled: self.r.chance(80),
                kind: DeviceKind::Builtin {
                    device: BuiltinDevice::Synth,
                },
                params: Default::default(),
                sidechain: None,
                pad: None,
            }),
        );
    }

    fn send(&mut self) {
        let (Some(from), Some(to)) = (
            self.r
                .pick(&self.tracks(&[TrackKind::Midi, TrackKind::Audio])),
            self.r.pick(&self.tracks(&[TrackKind::Return])),
        ) else {
            return;
        };
        let id = self.id();
        insert(
            &mut self.p,
            Entity::Send(TrackSend {
                id,
                from,
                to,
                level: Decibels(-6.0),
                pre_fader: self.r.chance(30),
            }),
        );
    }

    fn media(&mut self) -> MediaId {
        if let Some(m) = self.r.pick(&Self::keys(&self.p.media)) {
            return m;
        }
        let id: MediaId = self.id();
        insert(
            &mut self.p,
            Entity::Media(MediaRef {
                location: Default::default(),
                id,
                name: "a.wav".into(),
                file: format!("media/{id}.wav"),
                sample_rate: 48_000,
                channels: 2,
                frames: 96_000,
                hash: None,
            }),
        );
        id
    }

    fn clip(&mut self) {
        let Some(track) = self
            .r
            .pick(&self.tracks(&[TrackKind::Midi, TrackKind::Audio]))
        else {
            return;
        };
        let audio = self.p.tracks[&track].kind == TrackKind::Audio;
        let content = if audio {
            let media = self.media();
            ClipContent::Audio(AudioContent {
                media,
                gain: Decibels(-3.0),
                transpose: 0.0,
                fade_in: Beats(0.25),
                fade_out: Beats::ZERO,
                warp: WarpSettings {
                    enabled: self.r.chance(60),
                    mode: WarpMode::Complex,
                    source_bpm: self.r.chance(50).then_some(100.0),
                    formants: Default::default(),
                },
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::EqualPower,
                reversed: self.r.chance(20),
            })
        } else {
            ClipContent::Midi
        };
        // Take-lane clips (comping) on some tracks.
        let lane = if self.r.chance(20) {
            let lanes: Vec<TakeLaneId> = self.p.lanes_of(track).iter().map(|l| l.id).collect();
            match self.r.pick(&lanes) {
                Some(l) => Some(l),
                None => {
                    let id: TakeLaneId = self.id();
                    insert(
                        &mut self.p,
                        Entity::TakeLane(TakeLane {
                            id,
                            track,
                            order: OrderKey::between(None, None),
                            name: "Take 1".into(),
                            color: None,
                        }),
                    );
                    Some(id)
                }
            }
        } else {
            None
        };
        let id = self.id();
        let start = self.r.beats(32.0);
        insert(
            &mut self.p,
            Entity::Clip(Clip {
                lane,
                id,
                track,
                start,
                name: String::new(),
                color: None,
                muted: self.r.chance(10),
                length: Beats(1.0 + self.r.below(8) as f64),
                offset: self.r.beats(2.0),
                looping: ClipLoop {
                    enabled: self.r.chance(30),
                    start: Beats::ZERO,
                    end: Beats(4.0),
                },
                content,
            }),
        );
        if let Some(lane) = lane
            && self.r.chance(70)
        {
            let id = self.id();
            let s = start.0.floor();
            insert(
                &mut self.p,
                Entity::CompRegion(CompRegion {
                    id,
                    track,
                    lane,
                    start: Beats(s),
                    end: Beats(s + 1.0 + self.r.below(4) as f64),
                    crossfade: Seconds(0.01),
                }),
            );
        }
    }

    fn note(&mut self) {
        let midi: Vec<ClipId> = self
            .p
            .clips
            .values()
            .filter(|c| matches!(c.content, ClipContent::Midi))
            .map(|c| c.id)
            .collect();
        let Some(clip) = self.r.pick(&midi) else {
            return;
        };
        let note: NoteId = self.id();
        let start = self.r.beats(8.0);
        insert(
            &mut self.p,
            Entity::Note(Note {
                id: note,
                clip,
                pitch: 60 + self.r.below(4) as u8,
                velocity: 0.7,
                release_velocity: 0.5,
                start,
                duration: Beats(0.5),
                muted: self.r.chance(15),
            }),
        );
        if self.r.chance(15) {
            let id = self.id();
            insert(
                &mut self.p,
                Entity::NoteExpression(NoteExpression {
                    id,
                    note,
                    kind: NoteExpressionKind::Pressure,
                    points: vec![ExpressionPoint {
                        time: Beats::ZERO,
                        value: 0.5,
                        curve: CurveShape::Linear,
                    }],
                }),
            );
        }
        if self.r.chance(5) {
            let id = self.id();
            insert(
                &mut self.p,
                Entity::ExpressionLane(ExpressionLane {
                    id,
                    clip,
                    kind: ExpressionKind::PitchBend,
                    points: vec![ExpressionPoint {
                        time: Beats::ZERO,
                        value: 0.25,
                        curve: CurveShape::Linear,
                    }],
                }),
            );
        }
    }

    fn lane(&mut self) {
        let Some(track) = self.r.pick(&Self::keys(&self.p.tracks)) else {
            return;
        };
        let owner = match self.r.pick(&Self::keys(&self.p.clips)) {
            Some(clip) if self.r.chance(50) => AutomationOwner::Clip { clip },
            _ => AutomationOwner::Track { track },
        };
        // Clip envelopes mostly target their own track (others are skipped by compile).
        let own = match owner {
            AutomationOwner::Clip { clip } if self.r.chance(80) => self.p.clips[&clip].track,
            _ => track,
        };
        let target = match self.r.below(4) {
            0 => AutomationTarget::TrackVolume { track: own },
            1 => AutomationTarget::TrackPan { track: own },
            2 => match self.r.pick(&Self::keys(&self.p.sends)) {
                Some(send) => AutomationTarget::SendLevel { send },
                None => AutomationTarget::TrackVolume { track: own },
            },
            _ => match self.r.pick(&Self::keys(&self.p.devices)) {
                Some(device) => AutomationTarget::DeviceParam {
                    device,
                    param: ParamId(self.r.below(3) as u32),
                },
                None => AutomationTarget::TrackPan { track: own },
            },
        };
        let id = self.id();
        insert(
            &mut self.p,
            Entity::AutomationLane(AutomationLane {
                id,
                owner,
                target,
                enabled: self.r.chance(85),
            }),
        );
    }

    fn point(&mut self) {
        let Some(lane) = self.r.pick(&Self::keys(&self.p.automation_lanes)) else {
            return;
        };
        let id = self.id();
        let time = self.r.beats(16.0);
        insert(
            &mut self.p,
            Entity::AutomationPoint(AutomationPoint {
                id,
                lane,
                time,
                value: (self.r.below(5) as f64) / 4.0,
                curve: CurveShape::Linear,
            }),
        );
    }

    fn warp_marker(&mut self) {
        let audio: Vec<ClipId> = self
            .p
            .clips
            .values()
            .filter(|c| matches!(c.content, ClipContent::Audio(_)))
            .map(|c| c.id)
            .collect();
        let Some(clip) = self.r.pick(&audio) else {
            return;
        };
        let id = self.id();
        let beat = self.r.beats(8.0);
        insert(
            &mut self.p,
            Entity::WarpMarker(WarpMarker {
                id,
                clip,
                beat,
                source: Seconds(beat.0 * 0.4),
            }),
        );
    }

    fn step(&mut self) {
        match self.r.below(20) {
            0 | 1 => self.track(),
            2 => self.device(),
            3 => self.send(),
            4..=6 => self.clip(),
            7..=12 => self.note(),
            13 | 14 => self.lane(),
            15..=17 => self.point(),
            _ => self.warp_marker(),
        }
    }
}

fn random_project(seed: u64, steps: usize) -> Project {
    let mut ids = IdGen::new(seed);
    let now = 1_700_000_000_000;
    let p = Project::new(&mut ids, now);
    let mut g = Gen {
        p,
        ids,
        r: Rng(seed),
        now,
    };
    for _ in 0..steps {
        g.step();
    }
    if g.r.chance(50) {
        g.p.apply(&Op::Settings {
            change: SettingsChange::Swing(0.6),
        })
        .unwrap();
    }
    g.p.validate().unwrap();
    g.p
}

fn compile_both(p: &Project, seed: u64) -> (RenderGraphDescPair, usize) {
    // Some devices have engine nodes, some don't (skipped from chains, their automation
    // unresolvable).
    let nodes = move |d: DeviceId| {
        let h = d.0.0 as u64 ^ seed;
        (!h.is_multiple_of(3)).then_some(NodeKey {
            index: (h % 1000) as u32,
            generation: 1,
        })
    };
    let armed = move |t: TrackId| (t.0.0 as u64 ^ seed).is_multiple_of(4);
    let ctx = CompileContext {
        nodes: &nodes,
        descriptors: &builtin_descriptors,
        armed: &armed,
        version: 3,
    };
    let new = compile_graph_with(p, &ctx);
    let mut scanned = p.clone();
    scanned.invalidate_index();
    let old = super::reference::compile_graph_with(&scanned, &ctx);
    let notes = new
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(|c| match &c.content {
            ether_core::graph::ClipContentDesc::Midi { notes } => notes.len(),
            _ => 0,
        })
        .sum();
    ((new, old), notes)
}

type RenderGraphDescPair = (ether_core::RenderGraphDesc, ether_core::RenderGraphDesc);

#[test]
fn one_pass_compile_matches_the_scanning_compiler() {
    let (mut notes, mut envelopes, mut comps, mut automation) = (0, 0, 0, 0);
    for seed in 0..300u64 {
        let p = random_project(seed, 40 + (seed as usize % 7) * 40);
        let ((new, old), n) = compile_both(&p, seed);
        assert_eq!(new, old, "seed {seed}");
        // Bit-identical floats too (`==` equates 0.0 and -0.0).
        assert_eq!(format!("{new:?}"), format!("{old:?}"), "seed {seed}");
        notes += n;
        for t in &new.tracks {
            automation += t.automation.len();
            envelopes += t.clips.iter().map(|c| c.envelopes.len()).sum::<usize>();
            comps += t
                .clips
                .iter()
                .filter(|c| !p.clips.contains_key(&c.id))
                .count();
        }
    }
    // The generator exercises what it claims to.
    assert!(
        notes > 1000 && envelopes > 20 && comps > 5 && automation > 50,
        "notes {notes}, envelopes {envelopes}, comps {comps}, automation {automation}"
    );
}
