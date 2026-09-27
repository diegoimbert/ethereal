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
            DrumPad => drum_pads
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
            EntityKey::Track(_) | EntityKey::Send(_) | EntityKey::Device(_) => self.check_routing(),
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
                self.require(key, EntityKey::Track(d.track))?;
                check_order(&d.order)?;
                if d.params.values().any(|v| !v.is_finite()) {
                    return Err(invalid("device param values must be finite"));
                }
                if let DeviceKind::Builtin {
                    device: BuiltinDevice::Sampler { sample, slices },
                } = &d.kind
                {
                    if let Some(m) = sample {
                        self.require(key, EntityKey::Media(*m))?;
                    }
                    check_slices(slices)?;
                }
                if let Some(src) = d.sidechain {
                    self.require_track(key, src)?;
                    if src == d.track {
                        return Err(invariant("a device cannot sidechain its own track"));
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
                }
                Ok(())
            }
            EntityKey::Send(id) => {
                let s = &self.sends[&id];
                let from = self.require_track(key, s.from)?;
                let to = self.require_track(key, s.to)?;
                if to.kind != TrackKind::Return {
                    return Err(invariant("sends must target a return track"));
                }
                if from.kind == TrackKind::Master || s.from == s.to {
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
        }
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
        let key = EntityKey::Track(t.id);
        check_color(t.color)?;
        check_order(&t.order)?;
        check_db("track volume", t.mixer.volume)?;
        if !(-1.0..=1.0).contains(&t.mixer.pan.0) {
            return Err(invalid("pan must be in -1..=1"));
        }
        let top_level_only = matches!(t.kind, TrackKind::Master | TrackKind::Return);
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
            TrackInput::Track { track } => {
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
        // A sidechain source is rendered before the track whose device listens to it.
        for d in self.devices.values() {
            if let Some(src) = d.sidechain {
                edges.entry(src).or_default().push(d.track);
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
                        || matches!(t.input, TrackInput::Track { track } if track == id)
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
                            matches!(
                                &d.kind,
                                DeviceKind::Builtin { device: BuiltinDevice::Sampler { sample: Some(m), .. } } if *m == id
                            )
                        })
                        .map(|d| EntityKey::Device(d.id))
                }),
            EntityKey::Note(_)
            | EntityKey::AutomationPoint(_)
            | EntityKey::TempoPoint(_)
            | EntityKey::TimeSignature(_)
            | EntityKey::WarpMarker(_)
            | EntityKey::Marker(_)
            | EntityKey::MidiMapping(_) => None,
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
