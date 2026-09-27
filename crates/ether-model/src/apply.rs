//! Op application, inverses and invariant checks for [`Project`].
//!
//! Every mutation goes through [`Project::apply`]:
//! 1. pre-checks that need the *old* state (removing an entity that still has dependents,
//!    removing the master track);
//! 2. a purely structural mutation ([`Project::apply_unchecked`]) that returns the inverse op;
//! 3. post-checks on the *new* state, restricted to what the op can affect (the touched
//!    entity's references and values, slot uniqueness, routing cycles, tempo/signature at 0).
//!    On failure the inverse is applied, so the project is unchanged.
//!
//! All checks are predicates on the resulting state and [`Project::validate`] runs the same
//! checks over the whole document, so: a project reached by successful ops always validates,
//! and the inverse of an applied op always applies (it leads back to a valid state).

use crate::*;

/// Calls `$m!` with the `Entity` variant → `Project` table mapping.
macro_rules! tables {
    ($m:ident) => {
        $m! {
            Track => tracks,
            Clip => clips,
            Note => notes,
            Device => devices,
            Send => sends,
            AutomationLane => automation_lanes,
            AutomationPoint => automation_points,
            TempoPoint => tempo_points,
            TimeSignature => time_signatures,
            WarpMarker => warp_markers,
            Media => media,
            Marker => markers,
            MidiMapping => midi_mappings,
            DrumPad => drum_pads,
            TakeLane => take_lanes,
            CompRegion => comp_regions,
            RackChain => rack_chains,
            Modulator => modulators,
            ModMapping => mod_mappings
        }
    };
}

fn invalid(msg: impl Into<String>) -> ModelError {
    ModelError::InvalidValue(msg.into())
}

fn invariant(msg: impl Into<String>) -> ModelError {
    ModelError::Invariant(msg.into())
}

fn finite(x: f64) -> bool {
    x.is_finite()
}

fn check_color(c: Color) -> Result<(), ModelError> {
    if c.0 > 0xFF_FFFF {
        return Err(invalid(format!("color {:#x} is not 0xRRGGBB", c.0)));
    }
    Ok(())
}

fn check_order(k: &OrderKey) -> Result<(), ModelError> {
    if !k.is_valid() {
        return Err(invalid(format!("malformed order key {:?}", k.0)));
    }
    Ok(())
}

fn check_beats_nonneg(what: &str, b: Beats) -> Result<(), ModelError> {
    if !finite(b.0) || b.0 < 0.0 {
        return Err(invalid(format!(
            "{what} must be a finite beat >= 0, got {}",
            b.0
        )));
    }
    Ok(())
}

fn check_db(what: &str, d: Decibels) -> Result<(), ModelError> {
    if !d.0.is_finite() {
        return Err(invalid(format!("{what} must be finite")));
    }
    Ok(())
}

fn check_unit(what: &str, x: f64) -> Result<(), ModelError> {
    if !(0.0..=1.0).contains(&x) {
        return Err(invalid(format!("{what} must be in 0..=1, got {x}")));
    }
    Ok(())
}

pub(crate) fn check_signature(s: TimeSignature) -> Result<(), ModelError> {
    if s.numerator == 0 || !matches!(s.denominator, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(invalid(format!(
            "invalid time signature {}/{}",
            s.numerator, s.denominator
        )));
    }
    Ok(())
}

fn check_bpm(bpm: f64) -> Result<(), ModelError> {
    if !(finite(bpm) && bpm > 0.0 && bpm <= 999.0) {
        return Err(invalid(format!("bpm must be in (0, 999], got {bpm}")));
    }
    Ok(())
}

pub(crate) fn check_settings(s: &ProjectSettings) -> Result<(), ModelError> {
    if s.scale.root > 11 {
        return Err(invalid("scale root must be in 0..=11"));
    }
    let r = s.loop_region;
    check_beats_nonneg("loop start", r.start)?;
    if !(finite(r.end.0) && r.end.0 > r.start.0) {
        return Err(invalid("loop region end must be after its start"));
    }
    check_db("metronome volume", s.metronome_volume)?;
    check_unit("swing", f64::from(s.swing))?;
    if !(finite(s.swing_grid.0) && s.swing_grid.0 > 0.0) {
        return Err(invalid("swing grid must be > 0"));
    }
    Ok(())
}

fn check_tension(t: f32) -> Result<(), ModelError> {
    if !(-1.0..=1.0).contains(&t) {
        return Err(invalid("curve tension must be in -1..=1"));
    }
    Ok(())
}

fn check_fade_curve(c: FadeCurve) -> Result<(), ModelError> {
    match c {
        FadeCurve::Curve { tension } => check_tension(tension),
        FadeCurve::Linear | FadeCurve::EqualPower => Ok(()),
    }
}

fn check_slices(s: &SliceSettings) -> Result<(), ModelError> {
    if s.base_note > 127 {
        return Err(invalid("slice base note must be 0..=127"));
    }
    let mut prev = -1.0;
    for m in &s.markers {
        if !(finite(m.0) && m.0 >= 0.0 && m.0 > prev) {
            return Err(invalid(
                "slice markers must be finite, >= 0, sorted and distinct",
            ));
        }
        prev = m.0;
    }
    Ok(())
}

fn check_midi_source(s: &MidiSource) -> Result<(), ModelError> {
    if s.channel.is_some_and(|c| c > 15) {
        return Err(invalid("MIDI channel must be 0..=15"));
    }
    match s.control {
        MidiControl::Cc { number: n } | MidiControl::Note { key: n } if n > 127 => {
            Err(invalid("MIDI controller/note number must be 0..=127"))
        }
        _ => Ok(()),
    }
}

fn check_media_path(file: &str) -> Result<(), ModelError> {
    let ok = file.starts_with("media/")
        && file.len() > "media/".len()
        && !file.contains('\\')
        && !file
            .split('/')
            .any(|seg| seg == ".." || seg == "." || seg.is_empty());
    if !ok {
        return Err(invalid(format!(
            "media file must be a relative path under media/, got {file:?}"
        )));
    }
    Ok(())
}

fn check_zone(what: &str, z: Zone) -> Result<(), ModelError> {
    if z.lo > z.hi || z.hi > 127 {
        return Err(invalid(format!("{what} zone must satisfy lo <= hi <= 127")));
    }
    Ok(())
}

fn check_seconds_nonneg(what: &str, s: Seconds) -> Result<(), ModelError> {
    if !(finite(s.0) && s.0 >= 0.0) {
        return Err(invalid(format!("{what} must be finite and >= 0")));
    }
    Ok(())
}

fn check_sample_zones(zones: &[SampleZone]) -> Result<(), ModelError> {
    if zones.len() > MAX_ZONES {
        return Err(invalid(format!("at most {MAX_ZONES} sample zones")));
    }
    for z in zones {
        if z.root_key > 127 {
            return Err(invalid("zone root key must be 0..=127"));
        }
        if !(z.tune_cents.is_finite() && z.tune_cents.abs() <= 100.0) {
            return Err(invalid("zone tune must be within ±100 cents"));
        }
        check_zone("key", z.keys)?;
        check_zone("velocity", z.velocities)?;
        for (what, s) in [
            ("zone start", z.start),
            ("zone loop start", z.loop_start),
            ("zone loop end", z.loop_end),
            ("zone loop crossfade", z.loop_crossfade),
        ] {
            check_seconds_nonneg(what, s)?;
        }
        if let Some(end) = z.end {
            check_seconds_nonneg("zone end", end)?;
        }
        check_db("zone gain", z.gain)?;
        if !(-1.0..=1.0).contains(&z.pan.0) {
            return Err(invalid("pan must be in -1..=1"));
        }
    }
    Ok(())
}

fn is_rack(kind: &DeviceKind) -> bool {
    matches!(kind, DeviceKind::Builtin { device } if device.device_type().is_rack())
}

fn is_drum_rack(kind: &DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Builtin {
            device: BuiltinDevice::DrumRack
        }
    )
}

/// `true` if a MIDI mapping target references `track` directly.
fn mapping_refs_track(target: &MidiMapTarget, track: TrackId) -> bool {
    match target {
        MidiMapTarget::Param {
            target:
                AutomationTarget::TrackVolume { track: t } | AutomationTarget::TrackPan { track: t },
        }
        | MidiMapTarget::TrackMute { track: t }
        | MidiMapTarget::TrackSolo { track: t }
        | MidiMapTarget::TrackArm { track: t } => *t == track,
        _ => false,
    }
}

/// Swap `$field` with `$new` and return the old value wrapped in `$variant`.
macro_rules! swap {
    ($variant:path, $field:expr, $new:expr) => {
        $variant(std::mem::replace(&mut $field, $new))
    };
}

fn update_track(t: &mut Track, c: TrackChange) -> Result<TrackChange, ModelError> {
    use TrackChange as C;
    Ok(match c {
        C::Name(v) => swap!(C::Name, t.name, v),
        C::Color(v) => swap!(C::Color, t.color, v),
        C::Order(v) => swap!(C::Order, t.order, v),
        C::Parent(v) => swap!(C::Parent, t.parent, v),
        C::Volume(v) => swap!(C::Volume, t.mixer.volume, v),
        C::Pan(v) => swap!(C::Pan, t.mixer.pan, v),
        C::Mute(v) => swap!(C::Mute, t.mixer.mute, v),
        C::Solo(v) => swap!(C::Solo, t.mixer.solo, v),
        C::Input(v) => swap!(C::Input, t.input, v),
        C::Output(v) => swap!(C::Output, t.output, v),
        C::Monitor(v) => swap!(C::Monitor, t.monitor, v),
        C::Scale(v) => swap!(C::Scale, t.scale, v),
        C::Freeze(v) => swap!(C::Freeze, t.freeze, v),
        C::Vca(v) => swap!(C::Vca, t.vca, v),
    })
}

fn update_clip(c: &mut Clip, ch: ClipChange) -> Result<ClipChange, ModelError> {
    use ClipChange as C;
    fn audio(c: &mut Clip) -> Result<&mut AudioContent, ModelError> {
        match &mut c.content {
            ClipContent::Audio(a) => Ok(a),
            ClipContent::Midi => Err(invalid("audio-only clip field on a MIDI clip")),
        }
    }
    Ok(match ch {
        C::Track(v) => swap!(C::Track, c.track, v),
        C::Start(v) => swap!(C::Start, c.start, v),
        C::Name(v) => swap!(C::Name, c.name, v),
        C::Color(v) => swap!(C::Color, c.color, v),
        C::Muted(v) => swap!(C::Muted, c.muted, v),
        C::Length(v) => swap!(C::Length, c.length, v),
        C::Offset(v) => swap!(C::Offset, c.offset, v),
        C::Loop(v) => swap!(C::Loop, c.looping, v),
        C::Gain(v) => swap!(C::Gain, audio(c)?.gain, v),
        C::Transpose(v) => swap!(C::Transpose, audio(c)?.transpose, v),
        C::FadeIn(v) => swap!(C::FadeIn, audio(c)?.fade_in, v),
        C::FadeOut(v) => swap!(C::FadeOut, audio(c)?.fade_out, v),
        C::Warp(v) => swap!(C::Warp, audio(c)?.warp, v),
        C::FadeInCurve(v) => swap!(C::FadeInCurve, audio(c)?.fade_in_curve, v),
        C::FadeOutCurve(v) => swap!(C::FadeOutCurve, audio(c)?.fade_out_curve, v),
        C::Reversed(v) => swap!(C::Reversed, audio(c)?.reversed, v),
        C::Lane(v) => swap!(C::Lane, c.lane, v),
    })
}

fn update_note(n: &mut Note, c: NoteChange) -> Result<NoteChange, ModelError> {
    use NoteChange as C;
    Ok(match c {
        C::Pitch(v) => swap!(C::Pitch, n.pitch, v),
        C::Velocity(v) => swap!(C::Velocity, n.velocity, v),
        C::ReleaseVelocity(v) => swap!(C::ReleaseVelocity, n.release_velocity, v),
        C::Start(v) => swap!(C::Start, n.start, v),
        C::Duration(v) => swap!(C::Duration, n.duration, v),
        C::Muted(v) => swap!(C::Muted, n.muted, v),
    })
}

fn same_device_type(a: &DeviceKind, b: &DeviceKind) -> bool {
    match (a, b) {
        (DeviceKind::Builtin { device: a }, DeviceKind::Builtin { device: b }) => {
            a.device_type() == b.device_type()
        }
        (DeviceKind::Plugin { plugin: a }, DeviceKind::Plugin { plugin: b }) => {
            a.format == b.format && a.plugin_id == b.plugin_id
        }
        _ => false,
    }
}

fn update_device(d: &mut Device, c: DeviceChange) -> Result<DeviceChange, ModelError> {
    use DeviceChange as C;
    Ok(match c {
        C::Name(v) => swap!(C::Name, d.name, v),
        C::Enabled(v) => swap!(C::Enabled, d.enabled, v),
        C::Track(v) => swap!(C::Track, d.track, v),
        C::Order(v) => swap!(C::Order, d.order, v),
        C::Param { param, value } => {
            if value.is_some_and(|v| !v.is_finite()) {
                return Err(invalid("device param values must be finite"));
            }
            let old = match value {
                Some(v) => d.params.insert(param, v),
                None => d.params.remove(&param),
            };
            C::Param { param, value: old }
        }
        C::Kind(v) => {
            if !same_device_type(&d.kind, &v) {
                return Err(invalid("DeviceChange::Kind must keep the device type"));
            }
            swap!(C::Kind, d.kind, v)
        }
        C::Plugin(v) => match &mut d.kind {
            DeviceKind::Plugin { plugin } => {
                if plugin.format != v.format || plugin.plugin_id != v.plugin_id {
                    return Err(invalid("DeviceChange::Plugin must keep the plugin id"));
                }
                swap!(C::Plugin, *plugin, v)
            }
            DeviceKind::Builtin { .. } => {
                return Err(invalid("DeviceChange::Plugin on a built-in device"));
            }
        },
        C::Sidechain(v) => swap!(C::Sidechain, d.sidechain, v),
        C::Pad(v) => swap!(C::Pad, d.pad, v),
        C::Chain(v) => swap!(C::Chain, d.chain, v),
    })
}

fn update_send(s: &mut TrackSend, c: SendChange) -> Result<SendChange, ModelError> {
    use SendChange as C;
    Ok(match c {
        C::Level(v) => swap!(C::Level, s.level, v),
        C::PreFader(v) => swap!(C::PreFader, s.pre_fader, v),
    })
}

fn update_lane(
    l: &mut AutomationLane,
    c: AutomationLaneChange,
) -> Result<AutomationLaneChange, ModelError> {
    use AutomationLaneChange as C;
    Ok(match c {
        C::Enabled(v) => swap!(C::Enabled, l.enabled, v),
    })
}

fn update_point(
    p: &mut AutomationPoint,
    c: AutomationPointChange,
) -> Result<AutomationPointChange, ModelError> {
    use AutomationPointChange as C;
    Ok(match c {
        C::Time(v) => swap!(C::Time, p.time, v),
        C::Value(v) => swap!(C::Value, p.value, v),
        C::Curve(v) => swap!(C::Curve, p.curve, v),
    })
}

fn update_tempo(p: &mut TempoPoint, c: TempoPointChange) -> Result<TempoPointChange, ModelError> {
    use TempoPointChange as C;
    Ok(match c {
        C::Time(v) => swap!(C::Time, p.time, v),
        C::Bpm(v) => swap!(C::Bpm, p.bpm, v),
        C::Curve(v) => swap!(C::Curve, p.curve, v),
    })
}

fn update_signature(
    p: &mut TimeSignaturePoint,
    c: TimeSignatureChange,
) -> Result<TimeSignatureChange, ModelError> {
    use TimeSignatureChange as C;
    Ok(match c {
        C::Time(v) => swap!(C::Time, p.time, v),
        C::Signature(v) => swap!(C::Signature, p.signature, v),
    })
}

fn update_warp(m: &mut WarpMarker, c: WarpMarkerChange) -> Result<WarpMarkerChange, ModelError> {
    use WarpMarkerChange as C;
    Ok(match c {
        C::Beat(v) => swap!(C::Beat, m.beat, v),
        C::Source(v) => swap!(C::Source, m.source, v),
    })
}

fn update_media(m: &mut MediaRef, c: MediaChange) -> Result<MediaChange, ModelError> {
    use MediaChange as C;
    Ok(match c {
        C::Name(v) => swap!(C::Name, m.name, v),
        C::Location(v) => swap!(C::Location, m.location, v),
        C::Hash(v) => swap!(C::Hash, m.hash, v),
    })
}

fn update_take_lane(l: &mut TakeLane, c: TakeLaneChange) -> Result<TakeLaneChange, ModelError> {
    use TakeLaneChange as C;
    Ok(match c {
        C::Order(v) => swap!(C::Order, l.order, v),
        C::Name(v) => swap!(C::Name, l.name, v),
        C::Color(v) => swap!(C::Color, l.color, v),
    })
}

fn update_comp_region(
    r: &mut CompRegion,
    c: CompRegionChange,
) -> Result<CompRegionChange, ModelError> {
    use CompRegionChange as C;
    Ok(match c {
        C::Range(v) => {
            let old = BeatRange {
                start: r.start,
                end: r.end,
            };
            r.start = v.start;
            r.end = v.end;
            C::Range(old)
        }
        C::Lane(v) => swap!(C::Lane, r.lane, v),
        C::Crossfade(v) => swap!(C::Crossfade, r.crossfade, v),
    })
}

fn update_rack_chain(r: &mut RackChain, c: RackChainChange) -> Result<RackChainChange, ModelError> {
    use RackChainChange as C;
    Ok(match c {
        C::Order(v) => swap!(C::Order, r.order, v),
        C::Name(v) => swap!(C::Name, r.name, v),
        C::Color(v) => swap!(C::Color, r.color, v),
        C::Volume(v) => swap!(C::Volume, r.volume, v),
        C::Pan(v) => swap!(C::Pan, r.pan, v),
        C::Mute(v) => swap!(C::Mute, r.mute, v),
        C::Solo(v) => swap!(C::Solo, r.solo, v),
        C::Keys(v) => swap!(C::Keys, r.keys, v),
        C::Velocities(v) => swap!(C::Velocities, r.velocities, v),
        C::Select(v) => swap!(C::Select, r.select, v),
    })
}

fn update_modulator(m: &mut Modulator, c: ModulatorChange) -> Result<ModulatorChange, ModelError> {
    use ModulatorChange as C;
    Ok(match c {
        C::Order(v) => swap!(C::Order, m.order, v),
        C::Name(v) => swap!(C::Name, m.name, v),
        C::Param { param, value } => {
            if value.is_some_and(|v| !v.is_finite()) {
                return Err(invalid("modulator param values must be finite"));
            }
            let old = match value {
                Some(v) => m.params.insert(param, v),
                None => m.params.remove(&param),
            };
            C::Param { param, value: old }
        }
        C::Sidechain(v) => swap!(C::Sidechain, m.sidechain, v),
    })
}

fn update_mod_mapping(
    m: &mut ModMapping,
    c: ModMappingChange,
) -> Result<ModMappingChange, ModelError> {
    use ModMappingChange as C;
    Ok(match c {
        C::Depth(v) => swap!(C::Depth, m.depth, v),
        C::Source(v) => swap!(C::Source, m.source, v),
    })
}

fn update_marker(m: &mut Marker, c: MarkerChange) -> Result<MarkerChange, ModelError> {
    use MarkerChange as C;
    Ok(match c {
        C::Position(v) => swap!(C::Position, m.position, v),
        C::Name(v) => swap!(C::Name, m.name, v),
        C::Color(v) => swap!(C::Color, m.color, v),
    })
}

fn update_mapping(
    m: &mut MidiMapping,
    c: MidiMappingChange,
) -> Result<MidiMappingChange, ModelError> {
    use MidiMappingChange as C;
    Ok(match c {
        C::Source(v) => swap!(C::Source, m.source, v),
        C::Target(v) => swap!(C::Target, m.target, v),
        C::Min(v) => swap!(C::Min, m.min, v),
        C::Max(v) => swap!(C::Max, m.max, v),
        C::Mode(v) => swap!(C::Mode, m.mode, v),
    })
}

fn update_pad(p: &mut DrumPad, c: DrumPadChange) -> Result<DrumPadChange, ModelError> {
    use DrumPadChange as C;
    Ok(match c {
        C::Note(v) => swap!(C::Note, p.note, v),
        C::Name(v) => swap!(C::Name, p.name, v),
        C::Color(v) => swap!(C::Color, p.color, v),
        C::ChokeGroup(v) => swap!(C::ChokeGroup, p.choke_group, v),
        C::Volume(v) => swap!(C::Volume, p.volume, v),
        C::Pan(v) => swap!(C::Pan, p.pan, v),
        C::Mute(v) => swap!(C::Mute, p.mute, v),
    })
}

fn apply_settings(s: &mut ProjectSettings, c: SettingsChange) -> SettingsChange {
    use SettingsChange as C;
    match c {
        C::Name(v) => swap!(C::Name, s.name, v),
        C::LoopEnabled(v) => swap!(C::LoopEnabled, s.loop_enabled, v),
        C::LoopRegion(v) => swap!(C::LoopRegion, s.loop_region, v),
        C::Metronome(v) => swap!(C::Metronome, s.metronome, v),
        C::CountInBars(v) => swap!(C::CountInBars, s.count_in_bars, v),
        C::MetronomeVolume(v) => swap!(C::MetronomeVolume, s.metronome_volume, v),
        C::MetronomeAccent(v) => swap!(C::MetronomeAccent, s.metronome_accent, v),
        C::MetronomeSound(v) => swap!(C::MetronomeSound, s.metronome_sound, v),
        C::Swing(v) => swap!(C::Swing, s.swing, v),
        C::SwingGrid(v) => swap!(C::SwingGrid, s.swing_grid, v),
        C::Scale(v) => swap!(C::Scale, s.scale, v),
    }
}

impl EntityUpdate {
    /// Key of the entity this update changes.
    pub fn key(&self) -> EntityKey {
        match self {
            Self::Track { id, .. } => EntityKey::Track(*id),
            Self::Clip { id, .. } => EntityKey::Clip(*id),
            Self::Note { id, .. } => EntityKey::Note(*id),
            Self::Device { id, .. } => EntityKey::Device(*id),
            Self::Send { id, .. } => EntityKey::Send(*id),
            Self::AutomationLane { id, .. } => EntityKey::AutomationLane(*id),
            Self::AutomationPoint { id, .. } => EntityKey::AutomationPoint(*id),
            Self::TempoPoint { id, .. } => EntityKey::TempoPoint(*id),
            Self::TimeSignature { id, .. } => EntityKey::TimeSignature(*id),
            Self::WarpMarker { id, .. } => EntityKey::WarpMarker(*id),
            Self::Media { id, .. } => EntityKey::Media(*id),
            Self::Marker { id, .. } => EntityKey::Marker(*id),
            Self::MidiMapping { id, .. } => EntityKey::MidiMapping(*id),
            Self::DrumPad { id, .. } => EntityKey::DrumPad(*id),
            Self::TakeLane { id, .. } => EntityKey::TakeLane(*id),
            Self::CompRegion { id, .. } => EntityKey::CompRegion(*id),
            Self::RackChain { id, .. } => EntityKey::RackChain(*id),
            Self::Modulator { id, .. } => EntityKey::Modulator(*id),
            Self::ModMapping { id, .. } => EntityKey::ModMapping(*id),
        }
    }
}

impl Op {
    /// The entity this op touches (`None` for settings changes).
    pub fn key(&self) -> Option<EntityKey> {
        match self {
            Op::Insert { entity } => Some(entity.key()),
            Op::Remove { key } => Some(*key),
            Op::Update { update } => Some(update.key()),
            Op::Settings { .. } => None,
        }
    }
}

impl Project {
    pub(crate) fn contains(&self, key: EntityKey) -> bool {
        macro_rules! has {
            ($($v:ident => $t:ident),*) => {
                match key { $(EntityKey::$v(id) => self.$t.contains_key(&id),)* }
            };
        }
        tables!(has)
    }

    pub(crate) fn get_entity(&self, key: EntityKey) -> Option<Entity> {
        macro_rules! get {
            ($($v:ident => $t:ident),*) => {
                match key { $(EntityKey::$v(id) => self.$t.get(&id).cloned().map(Entity::$v),)* }
            };
        }
        tables!(get)
    }

    /// Insert or replace an entity without any check (patch mirrors).
    pub(crate) fn upsert_unchecked(&mut self, entity: Entity) {
        macro_rules! put {
            ($($v:ident => $t:ident),*) => {
                match entity { $(Entity::$v(e) => { self.$t.insert(e.id, e); })* }
            };
        }
        tables!(put)
    }

    /// Remove an entity without any check; returns it if it existed.
    pub(crate) fn remove_unchecked(&mut self, key: EntityKey) -> Option<Entity> {
        macro_rules! del {
            ($($v:ident => $t:ident),*) => {
                match key { $(EntityKey::$v(id) => self.$t.remove(&id).map(Entity::$v),)* }
            };
        }
        tables!(del)
    }

    /// Structural mutation only (existence checks, field-kind checks); returns the inverse.
    pub(crate) fn apply_unchecked(&mut self, op: &Op) -> Result<Op, ModelError> {
        match op {
            Op::Insert { entity } => {
                let key = entity.key();
                if self.contains(key) {
                    return Err(ModelError::AlreadyExists(key));
                }
                self.upsert_unchecked(entity.clone());
                Ok(Op::Remove { key })
            }
            Op::Remove { key } => {
                let entity = self
                    .remove_unchecked(*key)
                    .ok_or(ModelError::NotFound(*key))?;
                Ok(Op::Insert { entity })
            }
            Op::Update { update } => {
                let key = update.key();
                let nf = || ModelError::NotFound(key);
                use EntityUpdate as U;
                let inverse = match update.clone() {
                    U::Track { id, change } => U::Track {
                        id,
                        change: update_track(self.tracks.get_mut(&id).ok_or_else(nf)?, change)?,
                    },
                    U::Clip { id, change } => U::Clip {
                        id,
                        change: update_clip(self.clips.get_mut(&id).ok_or_else(nf)?, change)?,
                    },
                    U::Note { id, change } => U::Note {
                        id,
                        change: update_note(self.notes.get_mut(&id).ok_or_else(nf)?, change)?,
                    },
                    U::Device { id, change } => U::Device {
                        id,
                        change: update_device(self.devices.get_mut(&id).ok_or_else(nf)?, change)?,
                    },
                    U::Send { id, change } => U::Send {
                        id,
                        change: update_send(self.sends.get_mut(&id).ok_or_else(nf)?, change)?,
                    },
                    U::AutomationLane { id, change } => U::AutomationLane {
                        id,
                        change: update_lane(
                            self.automation_lanes.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::AutomationPoint { id, change } => U::AutomationPoint {
                        id,
                        change: update_point(
                            self.automation_points.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::TempoPoint { id, change } => U::TempoPoint {
                        id,
                        change: update_tempo(
                            self.tempo_points.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::TimeSignature { id, change } => U::TimeSignature {
                        id,
                        change: update_signature(
                            self.time_signatures.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::WarpMarker { id, change } => U::WarpMarker {
                        id,
                        change: update_warp(
                            self.warp_markers.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::Media { id, change } => U::Media {
                        id,
                        change: update_media(self.media.get_mut(&id).ok_or_else(nf)?, change)?,
                    },
                    U::Marker { id, change } => U::Marker {
                        id,
                        change: update_marker(self.markers.get_mut(&id).ok_or_else(nf)?, change)?,
                    },
                    U::MidiMapping { id, change } => U::MidiMapping {
                        id,
                        change: update_mapping(
                            self.midi_mappings.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::DrumPad { id, change } => U::DrumPad {
                        id,
                        change: update_pad(self.drum_pads.get_mut(&id).ok_or_else(nf)?, change)?,
                    },
                    U::TakeLane { id, change } => U::TakeLane {
                        id,
                        change: update_take_lane(
                            self.take_lanes.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::CompRegion { id, change } => U::CompRegion {
                        id,
                        change: update_comp_region(
                            self.comp_regions.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::RackChain { id, change } => U::RackChain {
                        id,
                        change: update_rack_chain(
                            self.rack_chains.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::Modulator { id, change } => U::Modulator {
                        id,
                        change: update_modulator(
                            self.modulators.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                    U::ModMapping { id, change } => U::ModMapping {
                        id,
                        change: update_mod_mapping(
                            self.mod_mappings.get_mut(&id).ok_or_else(nf)?,
                            change,
                        )?,
                    },
                };
                Ok(Op::Update { update: inverse })
            }
            Op::Settings { change } => Ok(Op::Settings {
                change: apply_settings(&mut self.settings, change.clone()),
            }),
        }
    }

    pub(crate) fn apply_checked(&mut self, op: &Op) -> Result<Op, ModelError> {
        if let Op::Remove { key } = op {
            if let EntityKey::Track(id) = key
                && self
                    .tracks
                    .get(id)
                    .is_some_and(|t| t.kind == TrackKind::Master)
            {
                return Err(invariant("the master track cannot be removed"));
            }
            if self.contains(*key) && self.first_dependent(*key).is_some() {
                return Err(ModelError::HasChildren(*key));
            }
        }
        let inverse = self.apply_unchecked(op)?;
        if let Err(e) = self.check_after(op) {
            self.apply_unchecked(&inverse)
                .expect("the inverse of a just-applied op always applies");
            return Err(e);
        }
        Ok(inverse)
    }

    pub(crate) fn apply_all_checked(&mut self, ops: &[Op]) -> Result<Vec<Op>, ModelError> {
        let mut inverses = Vec::with_capacity(ops.len());
        for op in ops {
            match self.apply_checked(op) {
                Ok(inv) => inverses.push(inv),
                Err(e) => {
                    for inv in inverses.iter().rev() {
                        self.apply_unchecked(inv)
                            .expect("rolling back a partially applied batch always succeeds");
                    }
                    return Err(e);
                }
            }
        }
        inverses.reverse();
        Ok(inverses)
    }

    fn check_after(&self, op: &Op) -> Result<(), ModelError> {
        match op {
            Op::Insert { entity } => {
                self.check_entity(entity.key())?;
                self.check_globals(entity.key())
            }
            Op::Update { update } => {
                self.check_entity(update.key())?;
                self.check_globals(update.key())
            }
            Op::Remove { key } => self.check_globals(*key),
            Op::Settings { .. } => check_settings(&self.settings),
        }
    }

    /// Document-wide invariants an op on `key`'s table can break.
    fn check_globals(&self, key: EntityKey) -> Result<(), ModelError> {
        match key {
            // Devices: sidechain edges are routing edges.
            EntityKey::Track(_)
            | EntityKey::Send(_)
            | EntityKey::Device(_)
            | EntityKey::Modulator(_) => self.check_routing(),
            EntityKey::TempoPoint(_) => {
                if !self
                    .tempo_points
                    .values()
                    .any(|p| p.time.approx_eq(Beats::ZERO))
                {
                    return Err(invariant("there must be a tempo point at beat 0"));
                }
                Ok(())
            }
            EntityKey::TimeSignature(_) => {
                if !self
                    .time_signatures
                    .values()
                    .any(|p| p.time.approx_eq(Beats::ZERO))
                {
                    return Err(invariant("there must be a time signature at beat 0"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn require(&self, entity: EntityKey, missing: EntityKey) -> Result<(), ModelError> {
        if self.contains(missing) {
            Ok(())
        } else {
            Err(ModelError::DanglingReference { entity, missing })
        }
    }

    fn require_track(&self, entity: EntityKey, id: TrackId) -> Result<&Track, ModelError> {
        self.tracks.get(&id).ok_or(ModelError::DanglingReference {
            entity,
            missing: EntityKey::Track(id),
        })
    }

    /// References and value constraints of one entity (which must exist).
    pub(crate) fn check_entity(&self, key: EntityKey) -> Result<(), ModelError> {
        match key {
            EntityKey::Track(id) => self.check_track(&self.tracks[&id]),
            EntityKey::Clip(id) => self.check_clip(&self.clips[&id]),
            EntityKey::Note(id) => {
                let n = &self.notes[&id];
                let clip = self
                    .clips
                    .get(&n.clip)
                    .ok_or(ModelError::DanglingReference {
                        entity: key,
                        missing: EntityKey::Clip(n.clip),
                    })?;
                if !matches!(clip.content, ClipContent::Midi) {
                    return Err(invariant("notes can only live in MIDI clips"));
                }
                if n.pitch > 127 {
                    return Err(invalid(format!("note pitch {} > 127", n.pitch)));
                }
                check_unit("velocity", f64::from(n.velocity))?;
                check_unit("release velocity", f64::from(n.release_velocity))?;
                check_beats_nonneg("note start", n.start)?;
                if !(finite(n.duration.0) && n.duration.0 > 0.0) {
                    return Err(invalid("note duration must be > 0"));
                }
                Ok(())
            }
            EntityKey::Device(id) => {
                let d = &self.devices[&id];
                if self.require_track(key, d.track)?.kind == TrackKind::Vca {
                    return Err(invariant("VCA tracks have no devices"));
                }
                check_order(&d.order)?;
                if d.params.values().any(|v| !v.is_finite()) {
                    return Err(invalid("device param values must be finite"));
                }
                if let DeviceKind::Builtin { device } = &d.kind {
                    for m in device.media() {
                        self.require(key, EntityKey::Media(m))?;
                    }
                    match device {
                        BuiltinDevice::Sampler { slices, .. } => check_slices(slices)?,
                        BuiltinDevice::MultiSampler { zones } => check_sample_zones(zones)?,
                        _ => {}
                    }
                }
                if let Some(src) = d.sidechain {
                    self.require_track(key, src)?;
                    if src == d.track {
                        return Err(invariant("a device cannot sidechain its own track"));
                    }
                    if d.pad.is_some() {
                        return Err(invariant("devices on drum pads cannot have a sidechain"));
                    }
                }
                // A rack's pad devices must stay on the rack's track (checked from the rack
                // too, so moving or re-kinding a rack can't strand its pad chains).
                if is_drum_rack(&d.kind)
                    && let Some(stray) = self.devices.values().find(|o| {
                        o.pad
                            .and_then(|p| self.drum_pads.get(&p))
                            .is_some_and(|p| p.rack == d.id)
                            && o.track != d.track
                    })
                {
                    return Err(invariant(format!(
                        "pad device {} is not on its drum rack's track",
                        stray.id
                    )));
                }
                if let Some(pad) = d.pad {
                    let pad = self
                        .drum_pads
                        .get(&pad)
                        .ok_or(ModelError::DanglingReference {
                            entity: key,
                            missing: EntityKey::DrumPad(pad),
                        })?;
                    if self.devices.get(&pad.rack).map(|r| r.track) != Some(d.track) {
                        return Err(invariant(
                            "a pad device must be on the same track as its drum rack",
                        ));
                    }
                    if is_drum_rack(&d.kind) {
                        return Err(invariant("drum racks cannot be nested in pads"));
                    }
                    if is_rack(&d.kind) {
                        return Err(invariant("racks cannot be nested in pads"));
                    }
                }
                self.check_device_v2(d)?;
                Ok(())
            }
            EntityKey::Send(id) => {
                let s = &self.sends[&id];
                let from = self.require_track(key, s.from)?;
                let to = self.require_track(key, s.to)?;
                if to.kind != TrackKind::Return {
                    return Err(invariant("sends must target a return track"));
                }
                if matches!(from.kind, TrackKind::Master | TrackKind::Vca) || s.from == s.to {
                    return Err(invariant("invalid send source"));
                }
                check_db("send level", s.level)
            }
            EntityKey::AutomationLane(id) => {
                let l = &self.automation_lanes[&id];
                match l.owner {
                    AutomationOwner::Track { track } => {
                        self.require(key, EntityKey::Track(track))?
                    }
                    AutomationOwner::Clip { clip } => self.require(key, EntityKey::Clip(clip))?,
                }
                self.check_target(key, &l.target)
            }
            EntityKey::AutomationPoint(id) => {
                let p = &self.automation_points[&id];
                self.require(key, EntityKey::AutomationLane(p.lane))?;
                check_beats_nonneg("automation point time", p.time)?;
                check_unit("automation value", p.value)?;
                if let CurveShape::Curve { tension } = p.curve {
                    check_tension(tension)?;
                }
                Ok(())
            }
            EntityKey::TempoPoint(id) => {
                let p = &self.tempo_points[&id];
                check_beats_nonneg("tempo point time", p.time)?;
                check_bpm(p.bpm)
            }
            EntityKey::TimeSignature(id) => {
                let p = &self.time_signatures[&id];
                check_beats_nonneg("time signature time", p.time)?;
                check_signature(p.signature)
            }
            EntityKey::WarpMarker(id) => {
                let m = &self.warp_markers[&id];
                let clip = self
                    .clips
                    .get(&m.clip)
                    .ok_or(ModelError::DanglingReference {
                        entity: key,
                        missing: EntityKey::Clip(m.clip),
                    })?;
                if !matches!(clip.content, ClipContent::Audio(_)) {
                    return Err(invariant("warp markers can only live in audio clips"));
                }
                if !(finite(m.beat.0) && finite(m.source.0) && m.source.0 >= 0.0) {
                    return Err(invalid("warp marker times must be finite (source >= 0)"));
                }
                Ok(())
            }
            EntityKey::Media(id) => {
                let m = &self.media[&id];
                check_media_path(&m.file)?;
                if let MediaLocation::External { path } = &m.location
                    && (path.is_empty() || path.contains('\0'))
                {
                    return Err(invalid("external media needs a path"));
                }
                if m.sample_rate == 0 || m.channels == 0 {
                    return Err(invalid("media needs a sample rate and channels"));
                }
                Ok(())
            }
            EntityKey::Marker(id) => {
                let m = &self.markers[&id];
                check_beats_nonneg("marker position", m.position)?;
                if let Some(c) = m.color {
                    check_color(c)?;
                }
                Ok(())
            }
            EntityKey::MidiMapping(id) => self.check_mapping(&self.midi_mappings[&id]),
            EntityKey::DrumPad(id) => self.check_pad(&self.drum_pads[&id]),
            EntityKey::TakeLane(id) => {
                let l = &self.take_lanes[&id];
                let track = self.require_track(key, l.track)?;
                if !matches!(track.kind, TrackKind::Audio | TrackKind::Midi) {
                    return Err(invariant("take lanes belong to audio or MIDI tracks"));
                }
                check_order(&l.order)?;
                if let Some(c) = l.color {
                    check_color(c)?;
                }
                Ok(())
            }
            EntityKey::CompRegion(id) => self.check_comp_region(&self.comp_regions[&id]),
            EntityKey::RackChain(id) => self.check_rack_chain(&self.rack_chains[&id]),
            EntityKey::Modulator(id) => {
                let m = &self.modulators[&id];
                self.require(key, EntityKey::Device(m.device))?;
                check_order(&m.order)?;
                let host = &self.devices[&m.device];
                if host.pad.is_some() || host.chain.is_some() {
                    return Err(invariant(
                        "modulators live on track-chain devices (not on drum pads or rack chains)",
                    ));
                }
                if let Some(src) = m.sidechain {
                    if m.kind != ModulatorKind::EnvelopeFollower {
                        return Err(invalid("only envelope followers take a sidechain"));
                    }
                    self.require_track(key, src)?;
                    if self.devices.get(&m.device).map(|d| d.track) == Some(src) {
                        return Err(invariant("a modulator cannot sidechain its own track"));
                    }
                }
                if m.params.values().any(|v| !v.is_finite()) {
                    return Err(invalid("modulator param values must be finite"));
                }
                Ok(())
            }
            EntityKey::ModMapping(id) => self.check_mod_mapping(&self.mod_mappings[&id]),
        }
    }

    /// v0.2 device rules: rack chains, nesting, mappings that target the device.
    fn check_device_v2(&self, d: &Device) -> Result<(), ModelError> {
        let key = EntityKey::Device(d.id);
        if let Some(chain) = d.chain {
            if d.pad.is_some() {
                return Err(invariant(
                    "a device cannot be on a drum pad and a rack chain",
                ));
            }
            let chain = self
                .rack_chains
                .get(&chain)
                .ok_or(ModelError::DanglingReference {
                    entity: key,
                    missing: EntityKey::RackChain(chain),
                })?;
            if self.devices.get(&chain.rack).map(|r| r.track) != Some(d.track) {
                return Err(invariant(
                    "a rack chain device must be on the same track as its rack",
                ));
            }
            if is_rack(&d.kind) || is_drum_rack(&d.kind) {
                return Err(invariant("racks cannot be nested in rack chains (v0.2)"));
            }
            if d.sidechain.is_some() {
                return Err(invariant("devices on rack chains cannot have a sidechain"));
            }
        }
        // A rack's chain devices must stay on the rack's track.
        if is_rack(&d.kind)
            && let Some(stray) = self.devices.values().find(|o| {
                o.chain
                    .and_then(|c| self.rack_chains.get(&c))
                    .is_some_and(|c| c.rack == d.id)
                    && o.track != d.track
            })
        {
            return Err(invariant(format!(
                "rack chain device {} is not on its rack's track",
                stray.id
            )));
        }
        // Modulators only on track-chain devices.
        if (d.pad.is_some() || d.chain.is_some())
            && self.modulators.values().any(|m| m.device == d.id)
        {
            return Err(invariant(
                "modulators live on track-chain devices (not on drum pads or rack chains)",
            ));
        }
        // Its modulators' sidechains stay off its own track when it moves.
        if self
            .modulators
            .values()
            .any(|m| m.device == d.id && m.sidechain == Some(d.track))
        {
            return Err(invariant("a modulator cannot sidechain its own track"));
        }
        // Mappings targeting this device stay in scope when it moves.
        for m in self.mod_mappings.values().filter(|m| m.device == d.id) {
            self.check_mod_mapping(m)?;
        }
        Ok(())
    }

    fn check_comp_region(&self, r: &CompRegion) -> Result<(), ModelError> {
        let key = EntityKey::CompRegion(r.id);
        self.require_track(key, r.track)?;
        let lane = self
            .take_lanes
            .get(&r.lane)
            .ok_or(ModelError::DanglingReference {
                entity: key,
                missing: EntityKey::TakeLane(r.lane),
            })?;
        if lane.track != r.track {
            return Err(invariant("a comp region's lane must belong to its track"));
        }
        check_beats_nonneg("comp region start", r.start)?;
        if !(finite(r.end.0) && r.end.0 > r.start.0 + Beats::EPSILON) {
            return Err(invalid("comp region end must be after its start"));
        }
        if !(finite(r.crossfade.0) && (0.0..=MAX_COMP_CROSSFADE).contains(&r.crossfade.0)) {
            return Err(invalid(format!(
                "comp crossfade must be 0..={MAX_COMP_CROSSFADE} s"
            )));
        }
        if self.comp_regions.values().any(|o| {
            o.id != r.id
                && o.track == r.track
                && o.start.0 < r.end.0 - Beats::EPSILON
                && r.start.0 < o.end.0 - Beats::EPSILON
        }) {
            return Err(invariant("comp regions of a track cannot overlap"));
        }
        Ok(())
    }

    fn check_rack_chain(&self, c: &RackChain) -> Result<(), ModelError> {
        let key = EntityKey::RackChain(c.id);
        let rack = self
            .devices
            .get(&c.rack)
            .ok_or(ModelError::DanglingReference {
                entity: key,
                missing: EntityKey::Device(c.rack),
            })?;
        if !is_rack(&rack.kind) || rack.pad.is_some() || rack.chain.is_some() {
            return Err(invariant("rack chains belong to a rack on a track chain"));
        }
        check_order(&c.order)?;
        if let Some(col) = c.color {
            check_color(col)?;
        }
        check_db("chain volume", c.volume)?;
        if !(-1.0..=1.0).contains(&c.pan.0) {
            return Err(invalid("pan must be in -1..=1"));
        }
        check_zone("key", c.keys)?;
        check_zone("velocity", c.velocities)?;
        check_zone("selector", c.select)
    }

    fn check_mod_mapping(&self, m: &ModMapping) -> Result<(), ModelError> {
        let key = EntityKey::ModMapping(m.id);
        if !(finite(m.depth) && (-1.0..=1.0).contains(&m.depth)) {
            return Err(invalid("modulation depth must be in -1..=1"));
        }
        let target = self
            .devices
            .get(&m.device)
            .ok_or(ModelError::DanglingReference {
                entity: key,
                missing: EntityKey::Device(m.device),
            })?;
        let in_rack = |rack: DeviceId| {
            target
                .chain
                .and_then(|c| self.rack_chains.get(&c))
                .is_some_and(|c| c.rack == rack)
        };
        match m.source {
            ModSource::Modulator { modulator } => {
                let host = self
                    .modulators
                    .get(&modulator)
                    .ok_or(ModelError::DanglingReference {
                        entity: key,
                        missing: EntityKey::Modulator(modulator),
                    })?
                    .device;
                if host != m.device && !in_rack(host) {
                    return Err(invariant(
                        "a modulator can only target its device or devices inside its rack",
                    ));
                }
                // No modulation of modulation: a rack's modulators don't drive its macros.
                if host == m.device && is_rack(&target.kind) && m.param.0 < RACK_SELECTOR_PARAM.0 {
                    return Err(invariant("modulators cannot target their rack's macros"));
                }
            }
            ModSource::Macro { rack, index } => {
                if index >= RACK_MACROS {
                    return Err(invalid(format!("macro index must be < {RACK_MACROS}")));
                }
                let r = self
                    .devices
                    .get(&rack)
                    .ok_or(ModelError::DanglingReference {
                        entity: key,
                        missing: EntityKey::Device(rack),
                    })?;
                if !is_rack(&r.kind) {
                    return Err(invariant("macro sources must be racks"));
                }
                let own = m.device == rack && m.param.0 >= RACK_SELECTOR_PARAM.0;
                if !own && !in_rack(rack) {
                    return Err(invariant(
                        "a macro can only target devices inside its rack (or the rack's non-macro params)",
                    ));
                }
            }
        }
        if self.mod_mappings.values().any(|o| {
            o.id != m.id && o.source == m.source && o.device == m.device && o.param == m.param
        }) {
            return Err(invariant("duplicate modulation mapping"));
        }
        Ok(())
    }

    fn check_pad(&self, p: &DrumPad) -> Result<(), ModelError> {
        let key = EntityKey::DrumPad(p.id);
        let rack = self
            .devices
            .get(&p.rack)
            .ok_or(ModelError::DanglingReference {
                entity: key,
                missing: EntityKey::Device(p.rack),
            })?;
        if !is_drum_rack(&rack.kind) || rack.pad.is_some() {
            return Err(invariant(
                "drum pads belong to a drum rack on a track chain",
            ));
        }
        if p.note > 127 {
            return Err(invalid("pad note must be 0..=127"));
        }
        if p.choke_group.is_some_and(|g| g == 0 || g > MAX_CHOKE_GROUP) {
            return Err(invalid(format!(
                "choke group must be 1..={MAX_CHOKE_GROUP}"
            )));
        }
        if let Some(c) = p.color {
            check_color(c)?;
        }
        check_db("pad volume", p.volume)?;
        if !(-1.0..=1.0).contains(&p.pan.0) {
            return Err(invalid("pan must be in -1..=1"));
        }
        if self
            .drum_pads
            .values()
            .any(|o| o.id != p.id && o.rack == p.rack && o.note == p.note)
        {
            return Err(invariant("two pads of a drum rack share a note"));
        }
        Ok(())
    }

    fn check_mapping(&self, m: &MidiMapping) -> Result<(), ModelError> {
        let key = EntityKey::MidiMapping(m.id);
        check_midi_source(&m.source)?;
        check_unit("mapping min", m.min)?;
        check_unit("mapping max", m.max)?;
        match &m.target {
            MidiMapTarget::Param { target } => self.check_target(key, target)?,
            MidiMapTarget::TrackMute { track }
            | MidiMapTarget::TrackSolo { track }
            | MidiMapTarget::TrackArm { track } => {
                self.require_track(key, *track)?;
            }
            MidiMapTarget::Transport { .. } => {}
        }
        if self
            .midi_mappings
            .values()
            .any(|o| o.id != m.id && o.source == m.source)
        {
            return Err(invariant("two MIDI mappings share a source"));
        }
        Ok(())
    }

    fn check_target(&self, key: EntityKey, target: &AutomationTarget) -> Result<(), ModelError> {
        match *target {
            AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track } => {
                self.require(key, EntityKey::Track(track))
            }
            AutomationTarget::SendLevel { send } => self.require(key, EntityKey::Send(send)),
            AutomationTarget::DeviceParam { device, .. } => {
                self.require(key, EntityKey::Device(device))
            }
        }
    }

    fn check_track(&self, t: &Track) -> Result<(), ModelError> {
        if let crate::scale::TrackScale::Custom { scale } = t.scale
            && scale.root > 11
        {
            return Err(invalid("scale root must be in 0..=11"));
        }
        if t.kind != TrackKind::Midi && t.scale != crate::scale::TrackScale::FollowProject {
            return Err(invalid("track scales are only available on MIDI tracks"));
        }
        let key = EntityKey::Track(t.id);
        if let Some(f) = &t.freeze {
            if !matches!(t.kind, TrackKind::Audio | TrackKind::Midi) {
                return Err(invariant("only audio and MIDI tracks can be frozen"));
            }
            let media = self
                .media
                .get(&f.media)
                .ok_or(ModelError::DanglingReference {
                    entity: key,
                    missing: EntityKey::Media(f.media),
                })?;
            if media.location != MediaLocation::Project {
                return Err(invariant("freeze renders are project media"));
            }
            check_seconds_nonneg("freeze start", f.start)?;
        }
        check_color(t.color)?;
        check_order(&t.order)?;
        check_db("track volume", t.mixer.volume)?;
        if !(-1.0..=1.0).contains(&t.mixer.pan.0) {
            return Err(invalid("pan must be in -1..=1"));
        }
        let top_level_only = matches!(
            t.kind,
            TrackKind::Master | TrackKind::Return | TrackKind::Vca
        );
        if let Some(vca) = t.vca {
            if t.kind == TrackKind::Master {
                return Err(invariant("the master track cannot be assigned to a VCA"));
            }
            if self.require_track(key, vca)?.kind != TrackKind::Vca {
                return Err(invariant("VCA assignments must target a VCA track"));
            }
            let mut cur = Some(vca);
            let mut steps = 0;
            while let Some(v) = cur {
                if v == t.id || steps > self.tracks.len() {
                    return Err(invariant("VCA assignment cycle"));
                }
                cur = self.tracks.get(&v).and_then(|v| v.vca);
                steps += 1;
            }
        }
        if t.kind == TrackKind::Vca && t.input != TrackInput::None {
            return Err(invariant("VCA tracks have no input"));
        }
        if t.kind == TrackKind::Master {
            if self
                .tracks
                .values()
                .any(|o| o.kind == TrackKind::Master && o.id != t.id)
            {
                return Err(invariant("there must be exactly one master track"));
            }
            if matches!(t.output, TrackOutput::Track { .. }) {
                return Err(invariant("the master track outputs to the hardware"));
            }
        }
        if let Some(parent) = t.parent {
            if top_level_only {
                return Err(invariant("master and return tracks are top-level"));
            }
            if self.require_track(key, parent)?.kind != TrackKind::Group {
                return Err(invariant("only group tracks can have children"));
            }
            // Parent chains must not cycle.
            let mut cur = Some(parent);
            let mut steps = 0;
            while let Some(p) = cur {
                if p == t.id || steps > self.tracks.len() {
                    return Err(invariant("track grouping cycle"));
                }
                cur = self.tracks.get(&p).and_then(|p| p.parent);
                steps += 1;
            }
        }
        match &t.input {
            TrackInput::Track { track, .. } => {
                self.require_track(key, *track)?;
                if *track == t.id {
                    return Err(invariant("a track cannot take its own output as input"));
                }
            }
            TrackInput::Audio { count, .. } if *count == 0 => {
                return Err(invalid("audio input needs at least one channel"));
            }
            TrackInput::Midi {
                channel: Some(ch), ..
            } if *ch > 15 => return Err(invalid("MIDI channel must be 0..=15")),
            _ => {}
        }
        if let TrackOutput::Track { track } = t.output {
            let dest = self.require_track(key, track)?;
            if !matches!(dest.kind, TrackKind::Group | TrackKind::Return) {
                return Err(invariant(
                    "tracks can only output to group or return tracks",
                ));
            }
        }
        Ok(())
    }

    fn check_clip(&self, c: &Clip) -> Result<(), ModelError> {
        let key = EntityKey::Clip(c.id);
        let track = self.require_track(key, c.track)?;
        match (&c.content, track.kind) {
            (ClipContent::Audio(a), TrackKind::Audio) => {
                self.require(key, EntityKey::Media(a.media))?;
                check_db("clip gain", a.gain)?;
                if !(a.transpose.is_finite() && a.transpose.abs() <= 48.0) {
                    return Err(invalid("transpose must be within ±48 semitones"));
                }
                check_beats_nonneg("fade in", a.fade_in)?;
                check_beats_nonneg("fade out", a.fade_out)?;
                check_fade_curve(a.fade_in_curve)?;
                check_fade_curve(a.fade_out_curve)?;
                if a.warp.source_bpm.is_some_and(|b| check_bpm(b).is_err()) {
                    return Err(invalid("invalid source bpm"));
                }
            }
            (ClipContent::Midi, TrackKind::Midi) => {}
            _ => return Err(invariant("clip content does not match its track kind")),
        }
        if let Some(lane) = c.lane {
            let l = self
                .take_lanes
                .get(&lane)
                .ok_or(ModelError::DanglingReference {
                    entity: key,
                    missing: EntityKey::TakeLane(lane),
                })?;
            if l.track != c.track {
                return Err(invariant("a take clip must be on its lane's track"));
            }
        }
        if !finite(c.start.0) {
            return Err(invalid("clip start must be finite"));
        }
        if let Some(col) = c.color {
            check_color(col)?;
        }
        if !(finite(c.length.0) && c.length.0 > 0.0) {
            return Err(invalid("clip length must be > 0"));
        }
        check_beats_nonneg("clip offset", c.offset)?;
        check_beats_nonneg("loop start", c.looping.start)?;
        if !(finite(c.looping.end.0) && c.looping.end.0 > c.looping.start.0) {
            return Err(invalid("clip loop end must be after its start"));
        }
        Ok(())
    }

    /// Where a track's post-fader signal goes (resolving `Default`).
    pub(crate) fn routing_destination(&self, t: &Track) -> Option<TrackId> {
        match t.output {
            TrackOutput::None => None,
            TrackOutput::Track { track } => Some(track),
            TrackOutput::Default if t.kind == TrackKind::Master => None,
            TrackOutput::Default => t.parent.or_else(|| {
                self.tracks
                    .values()
                    .find(|m| m.kind == TrackKind::Master)
                    .map(|m| m.id)
            }),
        }
    }

    /// The audio routing graph (outputs + sends) must be acyclic.
    fn check_routing(&self) -> Result<(), ModelError> {
        use std::collections::BTreeMap;
        let mut edges: BTreeMap<TrackId, Vec<TrackId>> = BTreeMap::new();
        for t in self.tracks.values() {
            if let Some(d) = self.routing_destination(t) {
                edges.entry(t.id).or_default().push(d);
            }
        }
        for s in self.sends.values() {
            edges.entry(s.from).or_default().push(s.to);
        }
        // v0.2: a track input from another track is rendered after its source.
        for t in self.tracks.values() {
            if let TrackInput::Track { track, .. } = t.input {
                edges.entry(track).or_default().push(t.id);
            }
        }
        // A sidechain source is rendered before the track whose device listens to it.
        for d in self.devices.values() {
            if let Some(src) = d.sidechain {
                edges.entry(src).or_default().push(d.track);
            }
        }
        // v0.2: envelope-follower sidechains too.
        for m in self.modulators.values() {
            if let (Some(src), Some(host)) = (m.sidechain, self.devices.get(&m.device)) {
                edges.entry(src).or_default().push(host.track);
            }
        }
        // 0 = unvisited, 1 = on stack, 2 = done. Iterative DFS.
        let mut state: BTreeMap<TrackId, u8> = BTreeMap::new();
        for &root in self.tracks.keys() {
            if state.contains_key(&root) {
                continue;
            }
            let mut stack = vec![(root, 0usize)];
            state.insert(root, 1);
            while let Some((node, i)) = stack.pop() {
                let next = edges.get(&node).and_then(|e| e.get(i)).copied();
                match next {
                    Some(n) => {
                        stack.push((node, i + 1));
                        match state.get(&n).copied().unwrap_or(0) {
                            1 => return Err(invariant("routing cycle")),
                            0 => {
                                state.insert(n, 1);
                                stack.push((n, 0));
                            }
                            _ => {}
                        }
                    }
                    None => {
                        state.insert(node, 2);
                    }
                }
            }
        }
        Ok(())
    }

    /// Some entity that references `key` (so `key` cannot be removed), if any.
    pub(crate) fn first_dependent(&self, key: EntityKey) -> Option<EntityKey> {
        match key {
            EntityKey::Track(id) => self
                .tracks
                .values()
                .find(|t| {
                    t.parent == Some(id)
                        || matches!(t.input, TrackInput::Track { track, .. } if track == id)
                        || t.vca == Some(id)
                        || matches!(t.output, TrackOutput::Track { track } if track == id)
                })
                .map(|t| EntityKey::Track(t.id))
                .or_else(|| {
                    self.clips
                        .values()
                        .find(|c| c.track == id)
                        .map(|c| EntityKey::Clip(c.id))
                })
                .or_else(|| {
                    self.devices
                        .values()
                        .find(|d| d.track == id)
                        .map(|d| EntityKey::Device(d.id))
                })
                .or_else(|| {
                    self.sends
                        .values()
                        .find(|s| s.from == id || s.to == id)
                        .map(|s| EntityKey::Send(s.id))
                })
                .or_else(|| {
                    self.automation_lanes
                        .values()
                        .find(|l| {
                            matches!(l.owner, AutomationOwner::Track { track } if track == id)
                                || matches!(
                                    l.target,
                                    AutomationTarget::TrackVolume { track }
                                        | AutomationTarget::TrackPan { track } if track == id
                                )
                        })
                        .map(|l| EntityKey::AutomationLane(l.id))
                })
                .or_else(|| {
                    self.devices
                        .values()
                        .find(|d| d.sidechain == Some(id))
                        .map(|d| EntityKey::Device(d.id))
                })
                .or_else(|| {
                    self.midi_mappings
                        .values()
                        .find(|m| mapping_refs_track(&m.target, id))
                        .map(|m| EntityKey::MidiMapping(m.id))
                })
                .or_else(|| {
                    self.comp_regions
                        .values()
                        .find(|r| r.track == id)
                        .map(|r| EntityKey::CompRegion(r.id))
                })
                .or_else(|| {
                    self.take_lanes
                        .values()
                        .find(|l| l.track == id)
                        .map(|l| EntityKey::TakeLane(l.id))
                })
                .or_else(|| {
                    self.modulators
                        .values()
                        .find(|m| m.sidechain == Some(id))
                        .map(|m| EntityKey::Modulator(m.id))
                }),
            EntityKey::Clip(id) => self
                .notes
                .values()
                .find(|n| n.clip == id)
                .map(|n| EntityKey::Note(n.id))
                .or_else(|| {
                    self.warp_markers
                        .values()
                        .find(|m| m.clip == id)
                        .map(|m| EntityKey::WarpMarker(m.id))
                })
                .or_else(|| {
                    self.automation_lanes
                        .values()
                        .find(|l| matches!(l.owner, AutomationOwner::Clip { clip } if clip == id))
                        .map(|l| EntityKey::AutomationLane(l.id))
                }),
            EntityKey::Device(id) => self
                .automation_lanes
                .values()
                .find(|l| matches!(l.target, AutomationTarget::DeviceParam { device, .. } if device == id))
                .map(|l| EntityKey::AutomationLane(l.id))
                .or_else(|| {
                    self.drum_pads
                        .values()
                        .find(|p| p.rack == id)
                        .map(|p| EntityKey::DrumPad(p.id))
                })
                .or_else(|| {
                    self.midi_mappings
                        .values()
                        .find(|m| matches!(&m.target, MidiMapTarget::Param { target: AutomationTarget::DeviceParam { device, .. } } if *device == id))
                        .map(|m| EntityKey::MidiMapping(m.id))
                })
                .or_else(|| {
                    self.mod_mappings
                        .values()
                        .find(|m| {
                            m.device == id
                                || matches!(m.source, ModSource::Macro { rack, .. } if rack == id)
                        })
                        .map(|m| EntityKey::ModMapping(m.id))
                })
                .or_else(|| {
                    self.modulators
                        .values()
                        .find(|m| m.device == id)
                        .map(|m| EntityKey::Modulator(m.id))
                })
                .or_else(|| {
                    self.rack_chains
                        .values()
                        .find(|c| c.rack == id)
                        .map(|c| EntityKey::RackChain(c.id))
                }),
            EntityKey::Send(id) => self
                .automation_lanes
                .values()
                .find(|l| matches!(l.target, AutomationTarget::SendLevel { send } if send == id))
                .map(|l| EntityKey::AutomationLane(l.id))
                .or_else(|| {
                    self.midi_mappings
                        .values()
                        .find(|m| matches!(&m.target, MidiMapTarget::Param { target: AutomationTarget::SendLevel { send } } if *send == id))
                        .map(|m| EntityKey::MidiMapping(m.id))
                }),
            EntityKey::DrumPad(id) => self
                .devices
                .values()
                .find(|d| d.pad == Some(id))
                .map(|d| EntityKey::Device(d.id)),
            EntityKey::AutomationLane(id) => self
                .automation_points
                .values()
                .find(|p| p.lane == id)
                .map(|p| EntityKey::AutomationPoint(p.id)),
            EntityKey::Media(id) => self
                .clips
                .values()
                .find(|c| matches!(&c.content, ClipContent::Audio(a) if a.media == id))
                .map(|c| EntityKey::Clip(c.id))
                .or_else(|| {
                    self.devices
                        .values()
                        .find(|d| {
                            matches!(&d.kind, DeviceKind::Builtin { device } if device.media().contains(&id))
                        })
                        .map(|d| EntityKey::Device(d.id))
                })
                .or_else(|| {
                    self.tracks
                        .values()
                        .find(|t| t.freeze.as_ref().is_some_and(|f| f.media == id))
                        .map(|t| EntityKey::Track(t.id))
                }),
            EntityKey::TakeLane(id) => self
                .clips
                .values()
                .find(|c| c.lane == Some(id))
                .map(|c| EntityKey::Clip(c.id))
                .or_else(|| {
                    self.comp_regions
                        .values()
                        .find(|r| r.lane == id)
                        .map(|r| EntityKey::CompRegion(r.id))
                }),
            EntityKey::RackChain(id) => self
                .devices
                .values()
                .find(|d| d.chain == Some(id))
                .map(|d| EntityKey::Device(d.id)),
            EntityKey::Modulator(id) => self
                .mod_mappings
                .values()
                .find(|m| m.source == ModSource::Modulator { modulator: id })
                .map(|m| EntityKey::ModMapping(m.id)),
            EntityKey::Note(_)
            | EntityKey::AutomationPoint(_)
            | EntityKey::TempoPoint(_)
            | EntityKey::TimeSignature(_)
            | EntityKey::WarpMarker(_)
            | EntityKey::Marker(_)
            | EntityKey::MidiMapping(_)
            | EntityKey::CompRegion(_)
            | EntityKey::ModMapping(_) => None,
        }
    }

    /// Full-document check (see [`Project::validate`]).
    pub(crate) fn validate_all(&self) -> Result<(), ModelError> {
        macro_rules! ids_match {
            ($($v:ident => $t:ident),*) => {$(
                for (id, e) in &self.$t {
                    if *id != e.id {
                        return Err(invariant(format!("{} table key {id} != entity id {}", stringify!($v), e.id)));
                    }
                    self.check_entity(EntityKey::$v(*id))?;
                }
            )*};
        }
        tables!(ids_match);
        let masters = self
            .tracks
            .values()
            .filter(|t| t.kind == TrackKind::Master)
            .count();
        if masters != 1 {
            return Err(invariant(format!(
                "expected exactly one master track, found {masters}"
            )));
        }
        check_settings(&self.settings)?;
        self.check_routing()?;
        self.check_globals(EntityKey::TempoPoint(TempoPointId::NIL))?;
        self.check_globals(EntityKey::TimeSignature(TimeSignatureId::NIL))?;
        Ok(())
    }
}
