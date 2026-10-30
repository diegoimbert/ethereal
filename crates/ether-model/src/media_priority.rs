//! Media priority: which media of a project matter most, so the ones about to be heard load
//! first when the files arrive one by one (a collaboration join, a shared copy opening).
//!
//! [`media_priority`] ranks **every** media of a project ([`Project::media`]):
//!
//! 1. **Tier** ([`MediaTier`]): heard on the arrangement first, then media only silent uses
//!    reach (muted tracks or clips, disabled devices, frozen-away clips and devices, muted
//!    drum pads and rack chains, take-lane clips no comp region plays), then media nothing
//!    triggers (sampler or multisampler zones and drum pads no note reaches, unused media):
//!    those are only needed if someone plays them live.
//! 2. **When it is first needed**, from the listening point ([`Listening`]), in widening
//!    windows: the next [`WINDOW_BEATS`] (4 bars of 4/4), the 3 windows after, the 7 after,
//!    and so on. What is about to play comes first. A looping transport region plays first
//!    and the rest of the song comes after a few passes of it; material before the playhead
//!    counts as if it came a while after the end of the song (someone will jump back to it).
//! 3. **Score**, within a window: the timeline time the media is heard (clip lengths, loops
//!    included; not the number of uses), each beat weighted by how soon it plays:
//!    `H / (H + d)` for a beat `d` beats away ([`HORIZON_BEATS`] = `H`). A sample heard
//!    once but for the whole song outranks a few short one-shots.
//! 4. **Size**: scores within [`TIE_RATIO`] of each other are a tie, broken by the smaller
//!    file first, so more tracks become ready sooner.
//!
//! Instruments: the notes routed to each sampler are mapped to what they trigger. A
//! `Sampler` is triggered by every note it receives; a `MultiSampler` zone by the notes in
//! its key × velocity range (round-robin siblings all count: they alternate on repeated
//! notes); a drum pad by the notes of its key (its chain hears them as
//! [`crate::PAD_PLAY_NOTE`]); an instrument or MIDI effect rack chain by the notes in its key
//! and velocity zones. Each note counts for its held time (at least [`MIN_HIT_BEATS`], so a
//! trigger count matters too). A MIDI effect that can change notes (arpeggiator, chord,
//! pitch, plugins, ...) before an instrument makes the mapping conservative from there on:
//! every zone and pad counts as triggered by every note. A MIDI sidechain reads its source
//! track's notes. Convolution impulse responses are needed whenever their track sounds.
//!
//! Pure and deterministic: the same project and listening point give the same order.

use std::collections::{BTreeMap, HashMap};

use crate::{
    BeatRange, Beats, BuiltinDevice, ClipContent, ClipId, Device, DeviceKind, IrSource, MediaId,
    MediaRef, Note, PAD_PLAY_NOTE, Project, Seconds, TrackId,
};

/// How fast the weight of a beat falls with its distance from the listening point: a beat
/// this many beats away weighs half of one playing now (two bars of 4/4).
pub const HORIZON_BEATS: f64 = 8.0;
/// The first window of "when it is first needed" (see the module docs).
pub const WINDOW_BEATS: f64 = 16.0;
/// Scores within this ratio are a tie (the smaller file goes first).
pub const TIE_RATIO: f64 = 1.25;
/// The least time a note counts for (a short hit still sounds its sample).
pub const MIN_HIT_BEATS: f64 = 0.25;
/// Passes of a looping transport region before the rest of the song counts.
const LOOP_PASSES: f64 = 4.0;
/// How far behind the end of the song material before the playhead counts (someone jumps
/// back to it; playback itself does not wrap).
const BEHIND_BEATS: f64 = 64.0;
/// Content loop repeats followed per note (later ones weigh next to nothing).
const MAX_LOOP_REPEATS: usize = 256;

/// How much a media is needed (see the module docs). Sorted most needed first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MediaTier {
    /// Heard on the arrangement.
    Heard,
    /// Only used where it is silent now (muted, disabled, frozen away, an uncomped take).
    Silent,
    /// Nothing triggers it (or nothing uses it): only live playing may need it.
    Idle,
}

/// Where listening starts: the playhead, and the transport loop when it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listening {
    pub at: Beats,
    pub loop_region: Option<BeatRange>,
}

impl Listening {
    /// From the playhead `at`, with `p`'s transport loop when it is enabled.
    pub fn of(p: &Project, at: Beats) -> Self {
        Self {
            at,
            loop_region: p.settings.loop_enabled.then_some(p.settings.loop_region),
        }
    }
}

/// One media's rank ([`media_priority`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MediaPriority {
    pub media: MediaId,
    /// [`MediaRef::file`].
    pub file: String,
    pub tier: MediaTier,
    /// The weighted heard time within its tier (beats; 0 for [`MediaTier::Idle`]).
    pub score: f64,
    /// Beats from the listening point to the first time it is needed (`None`: never on the
    /// arrangement).
    pub soonest: Option<f64>,
    /// Total timeline time it is heard (beats, loops included; overlapping uses add up).
    pub heard: f64,
    /// Bytes: the given size, else an estimate from its frames and channels.
    pub size: u64,
}

/// Every media of `p`, most needed first (see the module docs). `sizes`: file sizes in bytes
/// by [`MediaRef::file`] when known (others are estimated).
pub fn media_priority(
    p: &Project,
    listening: &Listening,
    sizes: &HashMap<String, u64>,
) -> Vec<MediaPriority> {
    let mut ranker = Ranker::new(p, listening);
    ranker.audio_clips();
    ranker.freezes();
    ranker.devices();
    let mut out: Vec<MediaPriority> = p.media.values().map(|m| ranker.result(m, sizes)).collect();
    out.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then(window(a.soonest).cmp(&window(b.soonest)))
            .then(bucket(b.score).cmp(&bucket(a.score)))
            .then(a.size.cmp(&b.size))
            .then(b.score.total_cmp(&a.score))
            .then_with(|| a.file.cmp(&b.file))
            .then(a.media.cmp(&b.media))
    });
    out
}

/// The files of [`media_priority`], each once (two media on one file count once, at the
/// better place).
pub fn file_order(p: &Project, listening: &Listening, sizes: &HashMap<String, u64>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    media_priority(p, listening, sizes)
        .into_iter()
        .filter(|m| seen.insert(m.file.clone()))
        .map(|m| m.file)
        .collect()
}

/// Which window (0 = the next [`WINDOW_BEATS`]) a media first needed `soonest` beats away
/// falls in: windows 1, 3, 7, ... windows long.
fn window(soonest: Option<f64>) -> u32 {
    match soonest {
        Some(d) if d.is_finite() => (1.0 + d.max(0.0) / WINDOW_BEATS).log2().floor() as u32,
        _ => u32::MAX,
    }
}

/// The tie bucket of a score (higher is better; 0 and below share the lowest).
fn bucket(score: f64) -> i64 {
    if score > 0.0 && score.is_finite() {
        (score.ln() / TIE_RATIO.ln()).floor() as i64
    } else {
        i64::MIN
    }
}

/// A note as heard on the timeline.
#[derive(Clone, Copy, Debug)]
struct Hit {
    /// Timeline beats.
    start: f64,
    end: f64,
    key: u8,
    /// 1..=127.
    velocity: u8,
    /// From a muted clip, a take lane or a muted track: it plays nothing now.
    silent: bool,
}

/// The notes reaching a device.
#[derive(Clone)]
struct Stream {
    hits: Vec<Hit>,
    /// Keys and velocities are as played (no note-changing MIDI effect on the way).
    exact: bool,
}

#[derive(Clone, Copy, Default)]
struct Use {
    score: f64,
    soonest: Option<f64>,
    heard: f64,
}

/// Per media, its uses per tier.
#[derive(Default)]
struct Acc {
    tiers: [Option<Use>; 3],
}

/// Distance on the listening timeline (see the module docs).
struct Clock {
    at: f64,
    /// The transport loop, when it is on and the playhead is before its end.
    looping: Option<(f64, f64)>,
    end: f64,
}

impl Clock {
    /// Beats from the listening point until timeline position `pos` plays.
    fn dist(&self, pos: f64) -> f64 {
        let at = self.at;
        match self.looping {
            None => {
                if pos >= at {
                    pos - at
                } else {
                    (self.end - at) + BEHIND_BEATS + pos
                }
            }
            Some((ls, le)) => {
                let first_pass = le - at;
                if pos >= at && pos < le {
                    pos - at
                } else if at >= ls && pos >= ls && pos < at {
                    first_pass + (pos - ls)
                } else {
                    let base = first_pass + LOOP_PASSES * (le - ls);
                    if pos >= le {
                        base + (pos - le)
                    } else {
                        base + (self.end - le) + BEHIND_BEATS + pos
                    }
                }
            }
        }
    }

    /// The weight of `[a, b)` (timeline beats) and how soon it starts playing.
    fn weigh(&self, a: f64, b: f64) -> (f64, f64) {
        if b.partial_cmp(&a) != Some(std::cmp::Ordering::Greater) {
            return (0.0, f64::INFINITY);
        }
        let mut cuts = vec![a, b, self.at];
        if let Some((ls, le)) = self.looping {
            cuts.extend([ls, le]);
        }
        cuts.retain(|x| *x >= a && *x <= b);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let h = HORIZON_BEATS;
        let (mut score, mut soonest) = (0.0, f64::INFINITY);
        for w in cuts.windows(2) {
            let (x, y) = (w[0], w[1]);
            let len = y - x;
            if len <= 0.0 {
                continue;
            }
            // Linear within a piece: measured at the middle to stay off the breakpoints.
            let d0 = (self.dist(x + len / 2.0) - len / 2.0).max(0.0);
            score += h * ((h + d0 + len) / (h + d0)).ln();
            soonest = soonest.min(d0);
        }
        (score, soonest)
    }
}

struct Ranker<'a> {
    p: &'a Project,
    clock: Clock,
    acc: BTreeMap<MediaId, Acc>,
    notes: HashMap<ClipId, Vec<&'a Note>>,
    track_hits: HashMap<TrackId, Vec<Hit>>,
}

impl<'a> Ranker<'a> {
    fn new(p: &'a Project, listening: &Listening) -> Self {
        let mut notes: HashMap<ClipId, Vec<&Note>> = HashMap::new();
        for n in p.notes.values() {
            notes.entry(n.clip).or_default().push(n);
        }
        let at = finite(listening.at.0).max(0.0);
        let looping = listening
            .loop_region
            .map(|r| (finite(r.start.0).max(0.0), finite(r.end.0)))
            .filter(|(ls, le)| le > ls && at < *le);
        let song_end = p
            .clips
            .values()
            .map(|c| finite(c.start.0 + c.length.0))
            .fold(0.0, f64::max);
        let end = song_end.max(at).max(looping.map_or(0.0, |l| l.1));
        Self {
            p,
            clock: Clock { at, looping, end },
            acc: BTreeMap::new(),
            notes,
            track_hits: HashMap::new(),
        }
    }

    fn add(&mut self, media: MediaId, tier: MediaTier, a: f64, b: f64) {
        let (score, soonest) = if tier == MediaTier::Idle {
            (0.0, f64::INFINITY)
        } else {
            self.clock.weigh(a, b)
        };
        let u = self.acc.entry(media).or_default().tiers[tier as usize].get_or_insert_default();
        u.score += score;
        u.heard += (b - a).max(0.0);
        if soonest.is_finite() {
            u.soonest = Some(u.soonest.map_or(soonest, |s| s.min(soonest)));
        }
    }

    fn idle(&mut self, media: MediaId) {
        self.acc.entry(media).or_default().tiers[MediaTier::Idle as usize].get_or_insert_default();
    }

    /// Muted itself, through a muted group or a muted VCA.
    fn track_muted(&self, track: TrackId) -> bool {
        let mut cur = Some(track);
        for _ in 0..=self.p.tracks.len() {
            let Some(t) = cur.and_then(|id| self.p.tracks.get(&id)) else {
                return false;
            };
            let vca_muted = t
                .vca
                .and_then(|v| self.p.tracks.get(&v))
                .is_some_and(|v| v.mixer.mute);
            if t.mixer.mute || vca_muted {
                return true;
            }
            cur = t.parent;
        }
        false
    }

    /// The track plays its clips (not muted, not covered by a freeze).
    fn track_live(&self, track: TrackId) -> bool {
        !self.track_muted(track) && self.p.frozen_by(track).is_none()
    }

    /// The timeline spans a clip plays: `[start, end)` minus a leading gap; a take-lane clip
    /// only where a comp region of its lane plays it.
    fn clip_spans(&self, c: &crate::Clip) -> Vec<(f64, f64)> {
        let a = c.start.0 + (-c.offset.0).max(0.0);
        let b = c.start.0 + c.length.0;
        match c.lane {
            None => vec![(a, b)],
            Some(lane) => self
                .p
                .comp_regions
                .values()
                .filter(|r| r.lane == lane)
                .map(|r| (r.start.0.max(a), r.end.0.min(b)))
                .filter(|(x, y)| y > x)
                .collect(),
        }
    }

    fn audio_clips(&mut self) {
        let p = self.p;
        for c in p.clips.values() {
            let ClipContent::Audio(audio) = &c.content else {
                continue;
            };
            let spans = self.clip_spans(c);
            let live = !c.muted && self.track_live(c.track) && !spans.is_empty();
            if live {
                for (a, b) in spans {
                    self.add(audio.media, MediaTier::Heard, a, b);
                }
            } else {
                // Silent now, ranked by where it would play.
                let a = c.start.0 + (-c.offset.0).max(0.0);
                self.add(audio.media, MediaTier::Silent, a, c.start.0 + c.length.0);
            }
        }
    }

    fn freezes(&mut self) {
        let p = self.p;
        let map = p.tempo_map();
        for t in p.tracks.values() {
            let Some(f) = &t.freeze else { continue };
            let start = map.seconds_to_beats(f.start).0;
            let end = match p.media.get(&f.media) {
                Some(m) if m.frames > 0 && m.sample_rate > 0 => {
                    let secs = f.start.0 + m.frames as f64 / m.sample_rate as f64;
                    map.seconds_to_beats(Seconds(secs)).0
                }
                _ => self.clock.end,
            };
            // A freeze inside a frozen group is inert.
            let tier = if self.track_muted(t.id) || p.frozen_ancestor(t.id).is_some() {
                MediaTier::Silent
            } else {
                MediaTier::Heard
            };
            self.add(f.media, tier, start, end);
        }
    }

    /// The notes `track`'s clips play (cached).
    fn hits_of(&mut self, track: TrackId) -> Vec<Hit> {
        if let Some(h) = self.track_hits.get(&track) {
            return h.clone();
        }
        let p = self.p;
        let track_silent = !self.track_live(track);
        let mut hits = Vec::new();
        for c in p.clips.values().filter(|c| c.track == track) {
            if !matches!(c.content, ClipContent::Midi) {
                continue;
            }
            let spans = self.clip_spans(c);
            let clip_silent = track_silent || c.muted || spans.is_empty();
            let Some(notes) = self.notes.get(&c.id) else {
                continue;
            };
            for n in notes {
                for (a, b) in note_spans(c, n) {
                    let heard = spans.iter().any(|(x, y)| a < *y && a >= *x);
                    hits.push(Hit {
                        start: a,
                        end: b,
                        key: n.pitch,
                        velocity: velocity(n.velocity),
                        silent: clip_silent || n.muted || !heard,
                    });
                }
            }
        }
        self.track_hits.insert(track, hits.clone());
        hits
    }

    /// When `track` sounds: its clips' spans (audio or notes), else the whole song (a bus).
    fn track_activity(&mut self, track: TrackId) -> Vec<(f64, f64, bool)> {
        let p = self.p;
        let silent = !self.track_live(track);
        let mut spans: Vec<(f64, f64, bool)> = p
            .clips
            .values()
            .filter(|c| c.track == track)
            .flat_map(|c| {
                self.clip_spans(c)
                    .into_iter()
                    .map(move |(a, b)| (a, b, silent || c.muted))
            })
            .collect();
        if spans.is_empty() {
            spans.push((0.0, self.clock.end.max(HORIZON_BEATS), silent));
        }
        spans
    }

    fn devices(&mut self) {
        let p = self.p;
        for t in p.tracks.values() {
            let stream = Stream {
                hits: self.hits_of(t.id),
                exact: true,
            };
            let chain: Vec<&Device> = p.devices_of(t.id);
            self.chain(t.id, &chain, stream, false);
        }
    }

    /// A device chain hearing `stream` (`silent`: the chain itself is muted or disabled).
    fn chain(&mut self, track: TrackId, devices: &[&'a Device], mut stream: Stream, silent: bool) {
        let p = self.p;
        for d in devices {
            let off = silent || !d.enabled || p.device_frozen(d.id);
            let input = match &d.midi_sidechain {
                Some(ms) => Stream {
                    hits: self.hits_of(ms.track),
                    exact: true,
                },
                None => stream.clone(),
            };
            match &d.kind {
                DeviceKind::Builtin { device } => match device {
                    BuiltinDevice::Sampler { sample, .. } => {
                        if let Some(m) = sample {
                            self.triggered(*m, &input.hits, off);
                        }
                    }
                    BuiltinDevice::MultiSampler { zones } => {
                        for z in zones {
                            let Some(m) = z.media else { continue };
                            let hits: Vec<Hit> = input
                                .hits
                                .iter()
                                .filter(|h| {
                                    !input.exact
                                        || (z.keys.contains(h.key)
                                            && z.velocities.contains(h.velocity))
                                })
                                .copied()
                                .collect();
                            self.triggered(m, &hits, off);
                        }
                    }
                    BuiltinDevice::DrumRack { .. } => {
                        for pad in p.pads_of(d.id) {
                            let hits: Vec<Hit> = input
                                .hits
                                .iter()
                                .filter(|h| !input.exact || h.key == pad.note)
                                .map(|h| Hit {
                                    key: PAD_PLAY_NOTE,
                                    ..*h
                                })
                                .collect();
                            let pad_stream = Stream {
                                hits,
                                exact: input.exact,
                            };
                            let devs = p.pad_devices_of(pad.id);
                            self.chain(track, &devs, pad_stream, off || pad.mute);
                        }
                    }
                    BuiltinDevice::InstrumentRack { .. }
                    | BuiltinDevice::MidiEffectRack { .. }
                    | BuiltinDevice::AudioEffectRack { .. } => {
                        let zoned = !matches!(device, BuiltinDevice::AudioEffectRack { .. });
                        for ch in p.chains_of(d.id) {
                            let hits: Vec<Hit> = input
                                .hits
                                .iter()
                                .filter(|h| {
                                    !zoned
                                        || !input.exact
                                        || (ch.keys.contains(h.key)
                                            && ch.velocities.contains(h.velocity))
                                })
                                .copied()
                                .collect();
                            let devs = p.chain_devices_of(ch.id);
                            let ch_stream = Stream {
                                hits,
                                exact: input.exact,
                            };
                            self.chain(track, &devs, ch_stream, off || ch.mute);
                        }
                    }
                    BuiltinDevice::ConvolutionReverb {
                        ir: Some(IrSource::Media { media }),
                    } => {
                        for (a, b, quiet) in self.track_activity(track) {
                            self.add(*media, tier(off || quiet), a, b);
                        }
                    }
                    other => {
                        for m in other.media() {
                            self.idle(m);
                        }
                    }
                },
                DeviceKind::Plugin { .. } | DeviceKind::Missing { .. } => {}
            }
            // What comes after a note-changing MIDI effect hears other notes.
            if d.enabled && changes_notes(&d.kind) {
                stream.exact = false;
            }
        }
    }

    /// `media` played by `hits` on a device (`off`: muted or disabled): each heard hit is a
    /// use; no hit at all leaves it idle.
    fn triggered(&mut self, media: MediaId, hits: &[Hit], off: bool) {
        if hits.is_empty() {
            self.idle(media);
            return;
        }
        for h in hits {
            self.add(media, tier(off || h.silent), h.start, h.end);
        }
    }

    fn result(&self, m: &MediaRef, sizes: &HashMap<String, u64>) -> MediaPriority {
        let acc = self.acc.get(&m.id);
        let best = acc.and_then(|a| {
            a.tiers
                .iter()
                .enumerate()
                .find_map(|(i, u)| u.map(|u| (i, u)))
        });
        let (tier, u) = match best {
            Some((0, u)) => (MediaTier::Heard, u),
            Some((1, u)) => (MediaTier::Silent, u),
            _ => (MediaTier::Idle, Use::default()),
        };
        let size = sizes.get(&m.file).copied().unwrap_or_else(|| {
            let estimate = m.frames.saturating_mul(u64::from(m.channels.max(1)) * 2);
            if estimate > 0 { estimate } else { u64::MAX / 2 }
        });
        MediaPriority {
            media: m.id,
            file: m.file.clone(),
            tier,
            score: u.score,
            soonest: u.soonest,
            heard: u.heard,
            size,
        }
    }
}

fn tier(silent: bool) -> MediaTier {
    if silent {
        MediaTier::Silent
    } else {
        MediaTier::Heard
    }
}

fn finite(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

/// A note velocity (0..=1) on the zones' 1..=127 scale.
fn velocity(v: f32) -> u8 {
    let v = if v.is_finite() { v } else { 1.0 };
    (v * 127.0).round().clamp(1.0, 127.0) as u8
}

/// MIDI effects that can change keys or velocities (or are opaque).
fn changes_notes(kind: &DeviceKind) -> bool {
    match kind {
        DeviceKind::Plugin { .. } => true,
        DeviceKind::Missing { .. } => false,
        DeviceKind::Builtin { device } => matches!(
            device,
            BuiltinDevice::Arpeggiator
                | BuiltinDevice::Chord
                | BuiltinDevice::ScaleQuantize
                | BuiltinDevice::NoteLength
                | BuiltinDevice::Velocity
                | BuiltinDevice::Randomizer
                | BuiltinDevice::Pitch
                | BuiltinDevice::NoteBlocker
                | BuiltinDevice::MidiEffectRack { .. }
        ),
    }
}

/// Where note `n` of clip `c` plays on the timeline: one span per pass of the clip's
/// content loop (bounded), each at least [`MIN_HIT_BEATS`] long and cut at the clip end.
fn note_spans(c: &crate::Clip, n: &Note) -> Vec<(f64, f64)> {
    let len = c.length.0;
    let s = n.start.0;
    let held = n.duration.0.max(MIN_HIT_BEATS);
    let mut out = Vec::new();
    let mut push = |d: f64| {
        if (0.0..len).contains(&d) {
            out.push((c.start.0 + d, c.start.0 + (d + held).min(len)));
        }
    };
    match c.played_loop() {
        None => push(s - c.offset.0),
        Some((ls, le)) => {
            if s >= c.offset.0 && s < le {
                push(s - c.offset.0);
            }
            if s >= ls && s < le {
                let first = le - c.offset.0;
                for k in 0..MAX_LOOP_REPEATS {
                    let d = first + k as f64 * (le - ls) + (s - ls);
                    if d >= len {
                        break;
                    }
                    push(d);
                }
            }
        }
    }
    out
}
