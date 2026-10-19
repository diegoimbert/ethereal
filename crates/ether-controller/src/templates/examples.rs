//! Example projects (#518): short loops that sound good at once and are simple to rebuild
//! from an empty project, offered as factory project templates (`factory/example-<slug>`,
//! the welcome screen's examples and the "New project" form). Each is built here the way a
//! user builds it: the same commands (tracks, devices, factory presets, pads, clips, notes,
//! tempo, swing, loop) applied in order, so the guides (`docs-site/content/welcome/examples`
//! and the guided tours) describe exactly this.
//!
//! Only built-in devices and their factory presets. The drums are drum rack pads, each
//! made by [`pad_sound`], the one place that decides what a pad plays: today a poly synth
//! with one of its drum presets (`poly-synth/kick`, …). When the default sample pack (#519)
//! ships, [`pad_sound`] gives each [`Drum`] its sample instead (a sampler pad, the
//! template's `samples` naming the files) and every example follows, unchanged.

use ether_core::protocol::Command;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::drum_rack::DrumRackCommand;
use ether_core::protocol::groove::GrooveCommand;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::template::{Template, TemplateBody};
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, internal};

/// Fixed clock for the examples' ids (2026-01-01), like the factory track templates.
const T: u64 = 1_767_225_600_000;

/// A drum sound of the examples' kit, on its General MIDI key (the drum rack's first bank
/// starts at C1 = 36).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drum {
    Kick,
    Snare,
    Clap,
    ClosedHat,
    OpenHat,
}

impl Drum {
    pub const ALL: [Drum; 5] = [
        Drum::Kick,
        Drum::Snare,
        Drum::Clap,
        Drum::ClosedHat,
        Drum::OpenHat,
    ];

    /// The pad's key: kick C1, snare D1, clap D#1, closed hat F#1, open hat A#1.
    pub fn note(self) -> u8 {
        match self {
            Drum::Kick => 36,
            Drum::Snare => 38,
            Drum::Clap => 39,
            Drum::ClosedHat => 42,
            Drum::OpenHat => 46,
        }
    }

    /// The pad's name (shown lowercase, like every name).
    pub fn name(self) -> &'static str {
        match self {
            Drum::Kick => "kick",
            Drum::Snare => "snare",
            Drum::Clap => "clap",
            Drum::ClosedHat => "closed hat",
            Drum::OpenHat => "open hat",
        }
    }
}

/// What a pad plays. Only presets for now; a sample variant comes with the sample pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadSound {
    /// A built-in instrument loaded with one of its factory presets.
    Preset {
        device: BuiltinDeviceType,
        preset: &'static str,
    },
}

/// The sound of each drum of the kit (see the module docs: the sample pack swaps it here).
pub fn pad_sound(drum: Drum) -> PadSound {
    let preset = match drum {
        Drum::Kick => "poly-synth/kick",
        Drum::Snare => "poly-synth/snare",
        Drum::Clap => "poly-synth/clap",
        Drum::ClosedHat => "poly-synth/closed-hat",
        Drum::OpenHat => "poly-synth/open-hat",
    };
    PadSound::Preset {
        device: BuiltinDeviceType::PolySynth,
        preset,
    }
}

/// One drum's line: 16 steps per bar (sixteenth notes), `x` a hit, `o` a soft hit, `.` a
/// rest; shorter than the clip, it repeats.
struct Line {
    drum: Drum,
    steps: &'static str,
}

/// A note or chord: start and length in beats, its pitches (C3 = 60), velocity 0..=1.
struct Hit {
    start: f64,
    length: f64,
    pitches: &'static [u8],
    velocity: f32,
}

const fn hit(start: f64, length: f64, pitches: &'static [u8]) -> Hit {
    Hit {
        start,
        length,
        pitches,
        velocity: 0.8,
    }
}

enum Part {
    Drums(Vec<Line>),
    Instrument {
        device: BuiltinDeviceType,
        preset: &'static str,
        notes: Vec<Hit>,
    },
}

struct Effect {
    device: BuiltinDeviceType,
    preset: &'static str,
}

struct ExampleTrack {
    name: &'static str,
    color: u32,
    /// The track fader (dB).
    volume: f32,
    part: Part,
    effects: Vec<Effect>,
}

pub struct Example {
    pub slug: &'static str,
    name: &'static str,
    description: &'static str,
    tags: &'static [&'static str],
    pub bpm: f64,
    /// Project swing (amount, grid in beats).
    swing: Option<(f32, f64)>,
    pub bars: u32,
    tracks: Vec<ExampleTrack>,
}

const DRUMS: u32 = 0xe6c07e;
const BASS: u32 = 0x8fa8e6;
const KEYS: u32 = 0xc39be6;

fn fx(device: BuiltinDeviceType, preset: &'static str) -> Effect {
    Effect { device, preset }
}

fn drums(volume: f32, lines: Vec<Line>, effects: Vec<Effect>) -> ExampleTrack {
    ExampleTrack {
        name: "drums",
        color: DRUMS,
        volume,
        part: Part::Drums(lines),
        effects,
    }
}

fn line(drum: Drum, steps: &'static str) -> Line {
    Line { drum, steps }
}

/// House: four on the floor, claps on 2 and 4, open hats on the off-beats, an off-beat bass.
fn house() -> Example {
    use Drum::*;
    Example {
        slug: "example-house-beat",
        name: "house beat",
        description: "four on the floor at 124 bpm: kick, clap, hats and an off-beat bass.",
        tags: &["example", "house", "drums"],
        bpm: 124.0,
        swing: None,
        bars: 2,
        tracks: vec![
            drums(
                -3.0,
                vec![
                    line(Kick, "x...x...x...x..."),
                    line(Clap, "....x.......x..."),
                    line(ClosedHat, ".o.o.o.o.o.o.o.o"),
                    line(OpenHat, "..x...x...x...x."),
                ],
                vec![],
            ),
            ExampleTrack {
                name: "bass",
                color: BASS,
                volume: -4.0,
                part: Part::Instrument {
                    device: BuiltinDeviceType::PolySynth,
                    preset: "poly-synth/deep-sub",
                    // A minor, then F and G: on every off-beat.
                    notes: vec![
                        hit(0.5, 0.25, &[45]),
                        hit(1.5, 0.25, &[45]),
                        hit(2.5, 0.25, &[45]),
                        hit(3.5, 0.25, &[45]),
                        hit(4.5, 0.25, &[41]),
                        hit(5.5, 0.25, &[41]),
                        hit(6.5, 0.25, &[43]),
                        hit(7.5, 0.25, &[43]),
                    ],
                },
                effects: vec![],
            },
        ],
    }
}

/// Hip-hop: a swung boom-bap beat under two soft seventh chords.
fn hip_hop() -> Example {
    use Drum::*;
    Example {
        slug: "example-hip-hop-loop",
        name: "hip-hop loop",
        description: "a swung boom-bap beat at 90 bpm under two soft keys chords.",
        tags: &["example", "hip-hop", "drums", "keys"],
        bpm: 90.0,
        swing: Some((0.5, 0.25)),
        bars: 2,
        tracks: vec![
            drums(
                -3.0,
                vec![
                    line(Kick, "x......x..x.....x.x.......x....."),
                    line(Snare, "....x.......x..."),
                    line(ClosedHat, "x.o.x.o.x.o.x.o."),
                ],
                vec![],
            ),
            ExampleTrack {
                name: "keys",
                color: KEYS,
                volume: 0.0,
                part: Part::Instrument {
                    device: BuiltinDeviceType::PolySynth,
                    preset: "poly-synth/soft-keys",
                    // C minor 7, then A-flat major 7.
                    notes: vec![
                        hit(0.0, 3.5, &[60, 63, 67, 70]),
                        hit(4.0, 3.5, &[56, 60, 63, 67]),
                    ],
                },
                effects: vec![fx(BuiltinDeviceType::Reverb, "reverb/small-room")],
            },
        ],
    }
}

/// Lo-fi: four seventh chords through chorus, bit crushing and reverb, a lazy beat under.
fn lofi() -> Example {
    use Drum::*;
    Example {
        slug: "example-lofi-chords",
        name: "lo-fi chords",
        description: "four warm seventh chords at 80 bpm, worn down by chorus, bit crushing and reverb, over a lazy beat.",
        tags: &["example", "lo-fi", "chords", "keys"],
        bpm: 80.0,
        swing: Some((0.35, 0.25)),
        bars: 4,
        tracks: vec![
            ExampleTrack {
                name: "chords",
                color: KEYS,
                volume: 4.0,
                part: Part::Instrument {
                    device: BuiltinDeviceType::PolySynth,
                    preset: "poly-synth/soft-keys",
                    // F major 7, E minor 7, D minor 7, C major 7: one bar each.
                    notes: vec![
                        hit(0.0, 3.75, &[53, 57, 60, 64]),
                        hit(4.0, 3.75, &[52, 55, 59, 62]),
                        hit(8.0, 3.75, &[50, 53, 57, 60]),
                        hit(12.0, 3.75, &[48, 52, 55, 59]),
                    ],
                },
                effects: vec![
                    fx(BuiltinDeviceType::Chorus, "chorus/classic-chorus"),
                    fx(BuiltinDeviceType::Bitcrusher, "bitcrusher/lo-fi-sampler"),
                    fx(BuiltinDeviceType::Reverb, "reverb/plate"),
                ],
            },
            drums(
                -8.0,
                vec![
                    line(Kick, "x.........x....."),
                    line(Snare, "....x.......x..."),
                    line(ClosedHat, "o.o.o.o.o.o.o.oo"),
                ],
                vec![],
            ),
        ],
    }
}

/// Bassline: a kick and hats, and an eighth-note acid bass in E minor on top.
fn bassline() -> Example {
    use Drum::*;
    Example {
        slug: "example-simple-bassline",
        name: "simple bassline",
        description: "an eighth-note acid bassline in E minor at 120 bpm over a kick and hats.",
        tags: &["example", "bass"],
        bpm: 120.0,
        swing: None,
        bars: 2,
        tracks: vec![
            drums(
                -4.0,
                vec![
                    line(Kick, "x...x...x...x..."),
                    line(ClosedHat, "..x...x...x...x."),
                ],
                vec![],
            ),
            ExampleTrack {
                name: "bass",
                color: BASS,
                volume: -4.0,
                part: Part::Instrument {
                    device: BuiltinDeviceType::PolySynth,
                    preset: "poly-synth/acid-bass",
                    notes: vec![
                        hit(0.0, 0.5, &[40]),
                        hit(0.75, 0.25, &[40]),
                        hit(1.5, 0.5, &[43]),
                        hit(2.0, 0.5, &[40]),
                        hit(2.75, 0.25, &[40]),
                        hit(3.0, 0.5, &[45]),
                        hit(3.5, 0.5, &[43]),
                        hit(4.0, 0.5, &[40]),
                        hit(4.75, 0.25, &[40]),
                        hit(5.5, 0.5, &[47]),
                        hit(6.0, 0.5, &[45]),
                        hit(6.5, 0.5, &[43]),
                        hit(7.0, 0.5, &[38]),
                        hit(7.5, 0.5, &[40]),
                    ],
                },
                effects: vec![],
            },
        ],
    }
}

/// Every example, in the order the welcome screen lists them.
pub fn all() -> Vec<Example> {
    vec![house(), hip_hop(), lofi(), bassline()]
}

/// The factory preset's params (`id` = `"<device-key>/<slug>"`).
fn preset_params(device: BuiltinDeviceType, id: &str) -> CmdResult<Vec<(ParamId, f64)>> {
    let fp = ether_devices::factory_presets(device)
        .iter()
        .find(|f| f.id == id)
        .ok_or_else(|| internal(format!("example: no factory preset {id}")))?;
    let preset =
        load_preset(fp.json).map_err(|e| internal(format!("example: preset {id}: {e}")))?;
    Ok(preset.params.into_iter().collect())
}

fn apply(ctx: &mut DocCtx, c: Command) -> CmdResult<()> {
    crate::doc::apply(ctx, &c).map(|_| ())
}

fn set_params(ctx: &mut DocCtx, device: DeviceId, params: &[(ParamId, f64)]) -> CmdResult<()> {
    for &(param, value) in params {
        apply(
            ctx,
            Command::Device(DeviceCommand::SetParam {
                device,
                param,
                value,
            }),
        )?;
    }
    Ok(())
}

fn insert_device(
    ctx: &mut DocCtx,
    track: TrackId,
    device: BuiltinDeviceType,
    preset: Option<&str>,
) -> CmdResult<DeviceId> {
    let id: DeviceId = ctx.new_id();
    apply(
        ctx,
        Command::Device(DeviceCommand::Insert {
            id,
            track,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(device),
            },
            before: None,
        }),
    )?;
    if let Some(p) = preset {
        set_params(ctx, id, &preset_params(device, p)?)?;
    }
    Ok(id)
}

/// The notes of drum lines over `beats` beats.
fn drum_notes(lines: &[Line], beats: f64) -> Vec<(u8, f64, f64, f32)> {
    let mut out = Vec::new();
    for l in lines {
        let steps: Vec<char> = l.steps.chars().collect();
        let count = (beats * 4.0) as usize;
        for i in 0..count {
            let velocity = match steps[i % steps.len()] {
                'x' => 0.9,
                'o' => 0.55,
                _ => continue,
            };
            out.push((l.drum.note(), i as f64 * 0.25, 0.25, velocity));
        }
    }
    out
}

/// Build `ex` into `ctx`'s project.
fn build(ctx: &mut DocCtx, ex: &Example) -> CmdResult<()> {
    let beats = f64::from(ex.bars) * 4.0;
    apply(
        ctx,
        Command::Transport(TransportCommand::SetTempo { bpm: ex.bpm }),
    )?;
    if let Some((amount, grid)) = ex.swing {
        apply(
            ctx,
            Command::Groove(GrooveCommand::SetSwing {
                amount,
                grid: Beats(grid),
            }),
        )?;
    }
    apply(
        ctx,
        Command::Transport(TransportCommand::SetLoopRegion {
            region: BeatRange {
                start: Beats::ZERO,
                end: Beats(beats),
            },
        }),
    )?;
    apply(
        ctx,
        Command::Transport(TransportCommand::SetLoopEnabled { enabled: true }),
    )?;
    for t in &ex.tracks {
        let track: TrackId = ctx.new_id();
        apply(
            ctx,
            Command::Track(TrackCommand::Create {
                id: track,
                kind: TrackKind::Midi,
                name: Some(t.name.into()),
                color: Some(Color(t.color)),
                parent: None,
                before: None,
            }),
        )?;
        apply(
            ctx,
            Command::Mixer(MixerCommand::SetVolume {
                track,
                volume: Decibels(t.volume),
            }),
        )?;
        let notes: Vec<(u8, f64, f64, f32)> = match &t.part {
            Part::Drums(lines) => {
                let rack = insert_device(ctx, track, BuiltinDeviceType::DrumRack, None)?;
                for drum in Drum::ALL {
                    if !lines.iter().any(|l| l.drum == drum) {
                        continue;
                    }
                    add_pad(ctx, rack, drum)?;
                }
                drum_notes(lines, beats)
            }
            Part::Instrument {
                device,
                preset,
                notes,
            } => {
                insert_device(ctx, track, *device, Some(preset))?;
                notes
                    .iter()
                    .flat_map(|h| {
                        h.pitches
                            .iter()
                            .map(move |p| (*p, h.start, h.length, h.velocity))
                    })
                    .collect()
            }
        };
        for e in &t.effects {
            insert_device(ctx, track, e.device, Some(e.preset))?;
        }
        let clip: ClipId = ctx.new_id();
        apply(
            ctx,
            Command::Clip(ClipCommand::CreateMidi {
                id: clip,
                track,
                start: Beats::ZERO,
                length: Beats(beats),
                name: Some(t.name.into()),
            }),
        )?;
        let notes = notes
            .into_iter()
            .map(|(pitch, start, length, velocity)| NoteSpec {
                id: ctx.new_id(),
                pitch,
                velocity,
                start: Beats(start),
                duration: Beats(length),
            })
            .collect();
        apply(ctx, Command::Note(NoteCommand::Add { clip, notes }))?;
    }
    Ok(())
}

/// A pad for `drum` on `rack`, playing [`pad_sound`].
fn add_pad(ctx: &mut DocCtx, rack: DeviceId, drum: Drum) -> CmdResult<()> {
    let pad: DrumPadId = ctx.new_id();
    apply(
        ctx,
        Command::DrumRack(DrumRackCommand::AddPad {
            id: pad,
            rack,
            note: drum.note(),
            name: Some(drum.name().into()),
        }),
    )?;
    match pad_sound(drum) {
        PadSound::Preset { device, preset } => {
            let id: DeviceId = ctx.new_id();
            apply(
                ctx,
                Command::DrumRack(DrumRackCommand::InsertDevice {
                    id,
                    pad,
                    device: DeviceSpec::Builtin {
                        device: BuiltinDevice::new(device),
                    },
                    before: None,
                }),
            )?;
            set_params(ctx, id, &preset_params(device, preset)?)?;
        }
    }
    Ok(())
}

/// The example as a project template (fixed ids, from a seed of its own).
pub fn template(ex: &Example, seed: u64) -> CmdResult<Template> {
    let mut ids = IdGen::new(seed);
    let mut project = Project::new(&mut ids, T);
    project.settings.name = ex.name.into();
    let mut host = super::NullHost;
    let mut ctx = DocCtx {
        tx: crate::tx::Tx::new(&mut project),
        ids: &mut ids,
        now: T,
        position: Beats::ZERO,
        host: &mut host,
        warnings: Vec::new(),
    };
    if let Err(e) = build(&mut ctx, ex) {
        ctx.tx.rollback();
        return Err(e);
    }
    // `finish` hands back the ops (unwinding them): the document gets them for good.
    let ops = ctx.tx.finish();
    for op in &ops {
        project
            .apply(op)
            .map_err(|e| internal(format!("example {}: {e}", ex.slug)))?;
    }
    Ok(Template {
        name: ex.name.into(),
        meta: PresetMeta {
            tags: ex.tags.iter().map(|t| t.to_string()).collect(),
            author: Some("ethereal".into()),
            description: Some(ex.description.into()),
        },
        body: TemplateBody::Project {
            project: Box::new(project),
            samples: Vec::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_build_valid_projects_with_their_parts() {
        let examples = all();
        assert!((3..=4).contains(&examples.len()));
        for (i, ex) in examples.iter().enumerate() {
            let t =
                template(ex, 0xE8_00 + i as u64).unwrap_or_else(|e| panic!("{}: {e:?}", ex.slug));
            let TemplateBody::Project { project: p, .. } = &t.body else {
                panic!("{}: not a project template", ex.slug)
            };
            p.validate().unwrap_or_else(|e| panic!("{}: {e}", ex.slug));
            assert!(p.settings.loop_enabled, "{}", ex.slug);
            assert_eq!(p.settings.loop_region.end, Beats(f64::from(ex.bars) * 4.0));
            assert_eq!(p.tempo_points.len(), 1);
            assert_eq!(p.tempo_points.values().next().unwrap().bpm, ex.bpm);
            // One MIDI track, one clip and notes per part; drums on pads that play.
            let midi: Vec<&Track> = p
                .tracks
                .values()
                .filter(|t| t.kind == TrackKind::Midi)
                .collect();
            assert_eq!(midi.len(), ex.tracks.len(), "{}", ex.slug);
            for t in &midi {
                let clips: Vec<&Clip> = p.clips.values().filter(|c| c.track == t.id).collect();
                assert_eq!(clips.len(), 1, "{} {}", ex.slug, t.name);
                assert!(
                    p.notes.values().any(|n| n.clip == clips[0].id),
                    "{} {}",
                    ex.slug,
                    t.name
                );
            }
            for pad in p.drum_pads.values() {
                assert!(
                    p.devices.values().any(|d| d.pad == Some(pad.id)),
                    "{}: pad {} plays nothing",
                    ex.slug,
                    pad.name
                );
            }
            // Deterministic: the same seed, the same document.
            assert_eq!(template(ex, 0xE8_00 + i as u64).unwrap(), t);
        }
    }

    #[test]
    fn drum_lines_repeat_over_the_clip() {
        let lines = [line(Drum::Kick, "x...x...x...x...")];
        let notes = drum_notes(&lines, 8.0);
        assert_eq!(notes.len(), 8);
        assert_eq!(notes[7].1, 7.0);
        let lines = [line(Drum::Kick, "x..............................o")];
        let notes = drum_notes(&lines, 8.0);
        assert_eq!(notes.len(), 2);
        assert_eq!((notes[1].1, notes[1].3), (7.75, 0.55));
    }
}
