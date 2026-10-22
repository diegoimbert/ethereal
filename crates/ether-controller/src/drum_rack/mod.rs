//! Drum racks and sampler slicing (roadmap v2; see CONTRACTS.md §11.12, `ether_model::drum_rack` and
//! `ether_protocol::drum_rack`).
//!
//! - `DrumRackCommand` / `SliceCommand` are document commands ([`rack_command`],
//!   [`slice_command`], from `doc::apply`): each is one undo step, creates are idempotent
//!   (client-chosen ids). `DrumRack::SetPadSolo` is the exception: runtime state kept in
//!   `EngineState::pad_solo` (no ops, not saved), compiled by [`apply_solo`] as mute of the
//!   rack's other pads.
//! - Deleting a rack deletes its pads and their devices first (`doc::DocCtx::delete_device`).
//! - [`racks_desc`] is called by `compile.rs` for every track: it compiles the track's
//!   racks into `TrackDesc::racks` (pad chains → node keys, pad mix).
//! - Slice edits only change the sampler's `slices`: `engine.rs` pushes them to the live
//!   node in place ([`updatable_in_place`] → `EngineBridge::update_builtin`), so sounding
//!   notes aren't cut; hosts without it re-create the node.

mod slicing;

use std::collections::{BTreeMap, BTreeSet};

use ether_core::RenderGraphDesc;
use ether_core::graph::{ChainEntry, PadDesc, RackDesc};
use ether_core::protocol::devices::DeviceSpec;
use ether_core::protocol::drum_rack::{AutoSlice, DrumRackCommand, SliceCommand, SlicePadIds};
use ether_core::protocol::model::*;
use ether_devices::sampler;

use crate::compile::CompileContext;
use crate::doc::{DocCtx, builtin_descriptor, default_device_name, order_before};
use crate::tx::{CmdResult, invalid, not_found};
use crate::{MAX_VOLUME_DB, SILENCE_DB};

pub use slicing::MAX_SLICES;

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// MIDI note name with C3 = 60 (Ableton convention): 36 → "C1".
pub fn note_name(note: u8) -> String {
    format!("{}{}", NOTE_NAMES[note as usize % 12], note as i32 / 12 - 2)
}

fn is_rack(d: &Device) -> bool {
    matches!(
        d.kind,
        DeviceKind::Builtin {
            device: BuiltinDevice::DrumRack
        }
    )
}

fn rack(ctx: &DocCtx, id: DeviceId) -> CmdResult<Device> {
    let d = ctx.device(id)?;
    if !is_rack(&d) || d.pad.is_some() {
        return Err(invalid(format!("device {id} is not a drum rack")));
    }
    Ok(d)
}

/// A chain rack (`device`: an existing one being moved, with what it holds) may go on a pad
/// of drum rack `rack`: one level below the drum rack, at most `MAX_RACK_DEPTH` deep (every
/// rack on the way down counts, `ether_model::rack`).
fn check_pad_depth(ctx: &DocCtx, rack: DeviceId, device: Option<DeviceId>) -> CmdResult<()> {
    let p = ctx.p();
    let level = p
        .device_path(rack)
        .map_or(MAX_RACK_DEPTH, |path| path.len())
        + 1;
    let below = device.map_or(0, |id| p.nested_rack_height(id));
    if level + below > MAX_RACK_DEPTH {
        return Err(invalid(format!("racks nest at most {MAX_RACK_DEPTH} deep")));
    }
    Ok(())
}

fn pad(ctx: &DocCtx, id: DrumPadId) -> CmdResult<DrumPad> {
    ctx.p()
        .drum_pads
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("drum pad {id}")))
}

fn check_note(note: u8) -> CmdResult<()> {
    if note > 127 {
        return Err(invalid(format!("invalid note {note} (0..=127)")));
    }
    Ok(())
}

fn set_pad(ctx: &mut DocCtx, id: DrumPadId, change: DrumPadChange) -> CmdResult<()> {
    ctx.tx.update(EntityUpdate::DrumPad { id, change })
}

/// Chain of `pad` (optionally without `except`) as order siblings.
fn pad_siblings(
    p: &Project,
    pad: DrumPadId,
    except: Option<DeviceId>,
) -> Vec<(OrderKey, DeviceId)> {
    p.pad_devices_of(pad)
        .into_iter()
        .filter(|d| Some(d.id) != except)
        .map(|d| (d.order.clone(), d.id))
        .collect()
}

fn track_siblings(p: &Project, track: TrackId, except: DeviceId) -> Vec<(OrderKey, DeviceId)> {
    p.devices_of(track)
        .into_iter()
        .filter(|d| d.id != except)
        .map(|d| (d.order.clone(), d.id))
        .collect()
}

fn add_pad(
    ctx: &mut DocCtx,
    id: DrumPadId,
    rack_id: DeviceId,
    note: u8,
    name: Option<String>,
) -> CmdResult<()> {
    check_note(note)?;
    let r = rack(ctx, rack_id)?;
    if ctx.p().pads_of(r.id).iter().any(|p| p.note == note) {
        return Err(invalid(format!("note {note} already has a pad")));
    }
    ctx.tx.insert(Entity::DrumPad(DrumPad {
        id,
        rack: r.id,
        note,
        name: name.unwrap_or_else(|| note_name(note)),
        color: None,
        choke_group: None,
        volume: Decibels::UNITY,
        pan: Pan(0.0),
        mute: false,
    }))
}

/// Default params of a built-in, overridden by `params`.
fn builtin_params(device: &BuiltinDevice, params: &[(ParamId, f64)]) -> BTreeMap<ParamId, f64> {
    let mut out: BTreeMap<ParamId, f64> = builtin_descriptor(device)
        .params
        .iter()
        .map(|p| (p.id, p.default))
        .collect();
    out.extend(params.iter().copied());
    out
}

pub fn rack_command(ctx: &mut DocCtx, c: &DrumRackCommand) -> CmdResult<()> {
    match c {
        DrumRackCommand::AddPad {
            id,
            rack,
            note,
            name,
        } => {
            if ctx.p().drum_pads.contains_key(id) {
                return Ok(());
            }
            add_pad(ctx, *id, *rack, *note, name.clone())
        }
        DrumRackCommand::RemovePad { id } => {
            pad(ctx, *id)?;
            ctx.delete_pad(*id)
        }
        DrumRackCommand::SetPadNote { id, note } => {
            check_note(*note)?;
            let p = pad(ctx, *id)?;
            if p.note == *note {
                return Ok(());
            }
            let pads: Vec<DrumPad> = ctx.p().pads_of(p.rack).into_iter().cloned().collect();
            match pads.iter().find(|o| o.note == *note) {
                // Swap: park the other pad on a free key first (each op is validated alone).
                Some(other) => {
                    let used: BTreeSet<u8> = pads.iter().map(|o| o.note).collect();
                    let free = (0..=127u8)
                        .find(|n| !used.contains(n))
                        .ok_or_else(|| invalid("every key of the rack has a pad"))?;
                    set_pad(ctx, other.id, DrumPadChange::Note(free))?;
                    set_pad(ctx, p.id, DrumPadChange::Note(*note))?;
                    set_pad(ctx, other.id, DrumPadChange::Note(p.note))
                }
                None => set_pad(ctx, p.id, DrumPadChange::Note(*note)),
            }
        }
        DrumRackCommand::RenamePad { id, name } => {
            pad(ctx, *id)?;
            set_pad(ctx, *id, DrumPadChange::Name(name.clone()))
        }
        DrumRackCommand::SetPadColor { id, color } => {
            pad(ctx, *id)?;
            set_pad(ctx, *id, DrumPadChange::Color(*color))
        }
        DrumRackCommand::SetChokeGroup { id, group } => {
            pad(ctx, *id)?;
            if group.is_some_and(|g| g == 0 || g > MAX_CHOKE_GROUP) {
                return Err(invalid(format!(
                    "choke group must be 1..={MAX_CHOKE_GROUP}"
                )));
            }
            set_pad(ctx, *id, DrumPadChange::ChokeGroup(*group))
        }
        DrumRackCommand::SetPadVolume { id, volume } => {
            pad(ctx, *id)?;
            if !volume.0.is_finite() {
                return Err(invalid("volume must be finite"));
            }
            let v = Decibels(volume.0.clamp(SILENCE_DB, MAX_VOLUME_DB));
            set_pad(ctx, *id, DrumPadChange::Volume(v))
        }
        DrumRackCommand::SetPadPan { id, pan } => {
            pad(ctx, *id)?;
            if !pan.0.is_finite() {
                return Err(invalid("pan must be finite"));
            }
            set_pad(ctx, *id, DrumPadChange::Pan(Pan(pan.0.clamp(-1.0, 1.0))))
        }
        DrumRackCommand::SetPadMute { id, mute } => {
            pad(ctx, *id)?;
            set_pad(ctx, *id, DrumPadChange::Mute(*mute))
        }
        DrumRackCommand::SetPadSolo { id, solo } => {
            pad(ctx, *id)?;
            ctx.host.set_pad_solo(*id, *solo);
            Ok(())
        }
        DrumRackCommand::InsertDevice {
            id,
            pad: pad_id,
            device,
            before,
        } => {
            if ctx.p().devices.contains_key(id) {
                return Ok(());
            }
            let kind = match device {
                DeviceSpec::Builtin { device } => DeviceKind::Builtin {
                    device: device.clone(),
                },
                DeviceSpec::Plugin {
                    plugin_id,
                    sandboxed,
                    format,
                    name: listed,
                } => DeviceKind::Plugin {
                    plugin: PluginInstance {
                        format: format.unwrap_or(PluginFormat::Clap),
                        plugin_id: plugin_id.clone(),
                        name: listed.clone().unwrap_or_else(|| plugin_id.clone()),
                        vendor: String::new(),
                        version: String::new(),
                        sandboxed: ctx.host.new_instance_sandboxed(
                            *sandboxed,
                            format.unwrap_or(PluginFormat::Clap),
                            plugin_id,
                        ),
                        always_process: false,
                        state: None,
                    },
                },
            };
            insert_pad_device(ctx, *id, *pad_id, kind, *before)?;
            if let DeviceSpec::Builtin { device } = device {
                crate::racks::add_first_chain(ctx, *id, device)?;
            }
            Ok(())
        }
        DrumRackCommand::MoveDevice {
            id,
            pad: to,
            before,
        } => {
            let d = ctx.device(*id)?;
            if *before == Some(d.id) {
                return Ok(());
            }
            match to {
                Some(pad_id) => {
                    let p = pad(ctx, *pad_id)?;
                    let r = rack(ctx, p.rack)?;
                    if r.track != d.track {
                        return Err(invalid("a pad device must be on the rack's track"));
                    }
                    if is_rack(&d) || ctx.p().has_nested_drum_rack(d.id) {
                        return Err(invalid("drum racks cannot be nested in pads"));
                    }
                    if d.kind.is_chain_rack() {
                        if ctx
                            .p()
                            .device_path(r.id)
                            .is_none_or(|path| path.contains(&d.id))
                        {
                            return Err(invalid("a rack cannot go inside itself"));
                        }
                        check_pad_depth(ctx, r.id, Some(d.id))?;
                    }
                    let order = order_before(&pad_siblings(ctx.p(), p.id, Some(d.id)), *before)?;
                    // A sidechain keyed from a chain of its own track might no longer be
                    // heard in time from the pad: cut it (others stay).
                    if crate::sidechain::depends_on_position(&d) {
                        crate::sidechain::cut(ctx, &d)?;
                        ctx.warnings.push(
                            crate::text::msg("notice.device.sidechain_removed")
                                .arg("device", d.name.as_str())
                                .into(),
                        );
                    }
                    if d.pad != Some(p.id) {
                        ctx.set_device(d.id, DeviceChange::Pad(Some(p.id)))?;
                    }
                    ctx.set_device(d.id, DeviceChange::Order(order))
                }
                None => {
                    let order = order_before(&track_siblings(ctx.p(), d.track, d.id), *before)?;
                    if d.pad.is_some() {
                        ctx.set_device(d.id, DeviceChange::Pad(None))?;
                    }
                    ctx.set_device(d.id, DeviceChange::Order(order))
                }
            }
        }
        DrumRackCommand::AddSamplePad {
            pad: pad_id,
            device,
            rack: rack_id,
            note,
            media,
        } => {
            if ctx.p().drum_pads.contains_key(pad_id) {
                return Ok(());
            }
            let m = ctx
                .p()
                .media
                .get(media)
                .cloned()
                .ok_or_else(|| not_found(format!("media {media}")))?;
            if ctx.p().devices.contains_key(device) {
                return Err(invalid(format!("device {device} already exists")));
            }
            let name = m
                .name
                .rsplit_once('.')
                .map_or(m.name.as_str(), |(stem, _)| stem)
                .to_owned();
            add_pad(ctx, *pad_id, *rack_id, *note, Some(name))?;
            let r = rack(ctx, *rack_id)?;
            let kind = BuiltinDevice::Sampler {
                sample: Some(*media),
                slices: SliceSettings::default(),
            };
            ctx.tx.insert(Entity::Device(Device {
                sidechain_tap: None,
                chain: None,
                id: *device,
                track: r.track,
                order: order_before::<DeviceId>(&[], None)?,
                name: default_device_name(&kind),
                enabled: true,
                params: builtin_params(&kind, &[]),
                kind: DeviceKind::Builtin { device: kind },
                sidechain: None,
                pad: Some(*pad_id),
            }))
        }
    }
}

/// Insert device `id` of `kind` into the chain of `pad_id` before `before` (None = end):
/// built-ins get their default params, plugins are instantiated (named after the
/// instance). Drum racks never go on pads; chain racks only within the nesting depth.
pub fn insert_pad_device(
    ctx: &mut DocCtx,
    id: DeviceId,
    pad_id: DrumPadId,
    kind: DeviceKind,
    before: Option<DeviceId>,
) -> CmdResult<Device> {
    let p = pad(ctx, pad_id)?;
    let r = rack(ctx, p.rack)?;
    let order = order_before(&pad_siblings(ctx.p(), p.id, None), before)?;
    let (kind, name, params) = match kind {
        DeviceKind::Missing { .. } => {
            return Err(invalid("a missing device can't be added to a pad"));
        }
        DeviceKind::Builtin { device } => {
            if matches!(device, BuiltinDevice::DrumRack) {
                return Err(invalid("drum racks cannot be nested in pads"));
            }
            if device.device_type().is_rack() {
                check_pad_depth(ctx, r.id, None)?;
            }
            let name = default_device_name(&device);
            let params = builtin_params(&device, &[]);
            (DeviceKind::Builtin { device }, name, params)
        }
        DeviceKind::Plugin { plugin } => {
            let desc = ctx.host.instantiate_plugin(id, &plugin)?;
            let name = desc
                .as_ref()
                .map(|d| d.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| plugin.name.clone());
            let plugin = PluginInstance {
                name: name.clone(),
                ..plugin
            };
            (DeviceKind::Plugin { plugin }, name, BTreeMap::new())
        }
    };
    let d = Device {
        sidechain_tap: None,
        chain: None,
        id,
        track: r.track,
        order,
        name,
        enabled: true,
        kind,
        params,
        sidechain: None,
        pad: Some(p.id),
    };
    ctx.tx.insert(Entity::Device(d.clone()))?;
    Ok(d)
}

/// The sampler device `id` and its sample/slices.
fn sampler_of(ctx: &DocCtx, id: DeviceId) -> CmdResult<(Device, Option<MediaId>, SliceSettings)> {
    let d = ctx.device(id)?;
    match &d.kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::Sampler { sample, slices },
        } => {
            let (sample, slices) = (*sample, slices.clone());
            Ok((d, sample, slices))
        }
        _ => Err(invalid(format!("device {id} is not a sampler"))),
    }
}

/// Length of the sampler's sample in seconds.
fn sample_seconds(ctx: &DocCtx, sample: Option<MediaId>) -> CmdResult<(MediaRef, f64)> {
    let media = sample.ok_or_else(|| {
        crate::tx::keyed_err(
            ether_core::protocol::ErrorCode::InvalidState,
            crate::text::msg("error.drum_rack.no_sample"),
        )
    })?;
    let m = ctx
        .p()
        .media
        .get(&media)
        .cloned()
        .ok_or_else(|| not_found(format!("media {media}")))?;
    let secs = m.frames as f64 / m.sample_rate.max(1) as f64;
    Ok((m, secs))
}

/// Sorted, finite, `>= 0`, deduplicated within 1 ms (the first wins), at most
/// [`MAX_SLICES`].
fn normalize(mut markers: Vec<f64>) -> Vec<Seconds> {
    markers.retain(|m| m.is_finite() && *m >= 0.0);
    markers.sort_by(f64::total_cmp);
    let mut out: Vec<Seconds> = Vec::with_capacity(markers.len());
    for m in markers {
        if out.last().is_none_or(|l| m - l.0 >= 0.001) && out.len() < MAX_SLICES {
            out.push(Seconds(m));
        }
    }
    out
}

/// Markers (source seconds, unsorted) of an automatic slicing of media `m` (`secs` long).
fn auto_markers(ctx: &DocCtx, m: &MediaRef, secs: f64, mode: AutoSlice) -> CmdResult<Vec<f64>> {
    Ok(match mode {
        AutoSlice::Transients { sensitivity } => {
            let peaks = ctx.host.peaks(m.id).ok_or_else(|| {
                crate::tx::keyed_err(
                    ether_core::protocol::ErrorCode::InvalidState,
                    crate::text::msg("error.drum_rack.sample_loading"),
                )
            })?;
            let env = slicing::envelope(peaks, m.id);
            slicing::transients(
                &env,
                ether_media::BASE_SAMPLES_PER_PEAK,
                m.sample_rate,
                sensitivity,
            )
        }
        AutoSlice::Grid { beats } => {
            if !(beats.is_finite() && beats > 0.0) {
                return Err(invalid("grid must be > 0 beats"));
            }
            let bpm = ctx.p().tempo_map().bpm_at(Beats(0.0));
            slicing::every(beats * 60.0 / bpm, secs)
        }
        AutoSlice::Equal { count } => {
            if count == 0 {
                return Err(invalid("slice count must be >= 1"));
            }
            let count = (count as usize).min(MAX_SLICES);
            slicing::every(secs / count as f64, secs)
        }
    })
}

pub fn slice_command(ctx: &mut DocCtx, c: &SliceCommand) -> CmdResult<()> {
    match c {
        SliceCommand::ToDrumRack { device, rack, pads } => {
            return to_drum_rack(ctx, *device, *rack, pads);
        }
        SliceCommand::SampleToPads {
            rack,
            media,
            mode,
            base_note,
            pads,
        } => return sample_to_pads(ctx, *rack, *media, *mode, *base_note, pads),
        _ => {}
    }
    let device = match c {
        SliceCommand::SetEnabled { device, .. }
        | SliceCommand::SetBaseNote { device, .. }
        | SliceCommand::Add { device, .. }
        | SliceCommand::Move { device, .. }
        | SliceCommand::Remove { device, .. }
        | SliceCommand::Auto { device, .. }
        | SliceCommand::ToDrumRack { device, .. } => *device,
        SliceCommand::SampleToPads { .. } => unreachable!("handled above"),
    };
    let (d, sample, s) = sampler_of(ctx, device)?;
    let markers = || s.markers.iter().map(|m| m.0).collect::<Vec<f64>>();
    let next = match c {
        SliceCommand::SetEnabled { enabled, .. } => SliceSettings {
            enabled: *enabled,
            ..s.clone()
        },
        SliceCommand::SetBaseNote { note, .. } => {
            check_note(*note)?;
            SliceSettings {
                base_note: *note,
                ..s.clone()
            }
        }
        SliceCommand::Add { positions, .. } => {
            if positions.iter().any(|p| !p.0.is_finite() || p.0 < 0.0) {
                return Err(invalid("slice positions must be finite and >= 0"));
            }
            let mut m = markers();
            // New markers never displace existing ones closer than 1 ms.
            for p in positions {
                if m.iter().all(|x| (x - p.0).abs() >= 0.001) {
                    m.push(p.0);
                }
            }
            SliceSettings {
                markers: normalize(m),
                ..s.clone()
            }
        }
        SliceCommand::Move {
            index, position, ..
        } => {
            let i = *index as usize;
            if i >= s.markers.len() {
                return Err(not_found(format!("slice {index}")));
            }
            if !position.0.is_finite() || position.0 < 0.0 {
                return Err(invalid("slice positions must be finite and >= 0"));
            }
            let mut m = markers();
            m.remove(i);
            // Moving onto another marker (within 1 ms) merges the two.
            if m.iter().all(|x| (x - position.0).abs() >= 0.001) {
                m.push(position.0);
            }
            SliceSettings {
                markers: normalize(m),
                ..s.clone()
            }
        }
        SliceCommand::Remove { indices, .. } => {
            let drop: BTreeSet<usize> = indices.iter().map(|&i| i as usize).collect();
            SliceSettings {
                markers: s
                    .markers
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !drop.contains(i))
                    .map(|(_, m)| *m)
                    .collect(),
                ..s.clone()
            }
        }
        SliceCommand::Auto { mode, .. } => {
            let (m, secs) = sample_seconds(ctx, sample)?;
            let found = auto_markers(ctx, &m, secs, *mode)?;
            SliceSettings {
                enabled: true,
                markers: normalize(found),
                ..s.clone()
            }
        }
        SliceCommand::ToDrumRack { .. } | SliceCommand::SampleToPads { .. } => {
            unreachable!("handled above")
        }
    };
    if next == s {
        return Ok(());
    }
    ctx.set_device(
        d.id,
        DeviceChange::Kind(DeviceKind::Builtin {
            device: BuiltinDevice::Sampler {
                sample,
                slices: next,
            },
        }),
    )
}

/// Checks the client-chosen ids of `count` slice pads: enough, new and distinct (and not
/// `rack`).
fn check_slice_ids(
    ctx: &DocCtx,
    ids: &[SlicePadIds],
    count: usize,
    rack: DeviceId,
) -> CmdResult<()> {
    if ids.len() < count {
        return Err(invalid(format!(
            "{count} slices need {count} pad ids ({} given)",
            ids.len()
        )));
    }
    let taken = ids[..count]
        .iter()
        .any(|i| ctx.p().drum_pads.contains_key(&i.pad) || ctx.p().devices.contains_key(&i.device));
    let distinct: BTreeSet<DeviceId> = ids[..count].iter().map(|i| i.device).collect();
    let distinct_pads: BTreeSet<DrumPadId> = ids[..count].iter().map(|i| i.pad).collect();
    if taken || distinct.len() != count || distinct_pads.len() != count || distinct.contains(&rack)
    {
        return Err(invalid("pad and device ids must be new and distinct"));
    }
    Ok(())
}

/// One slice pad per id (one per marker of `slices`, checked by the caller) on `rack`: note
/// `base_note + i`, named "Slice i+1", holding a sampler on `sample` (named `name`, with
/// `params` over the defaults) that plays just that slice at its original pitch.
#[allow(clippy::too_many_arguments)]
fn add_slice_pads(
    ctx: &mut DocCtx,
    rack_id: DeviceId,
    track: TrackId,
    sample: Option<MediaId>,
    secs: f64,
    slices: &SliceSettings,
    ids: &[SlicePadIds],
    name: &str,
    params: &BTreeMap<ParamId, f64>,
) -> CmdResult<()> {
    let percent = |t: f64| (t / secs * 100.0).clamp(0.0, 100.0);
    let pad_order = order_before::<DeviceId>(&[], None)?;
    for (i, pid) in ids.iter().enumerate() {
        let note = slices.base_note + i as u8;
        add_pad(
            ctx,
            pid.pad,
            rack_id,
            note,
            Some(format!("Slice {}", i + 1)),
        )?;
        let start = slices.markers[i].0;
        let end = slices.markers.get(i + 1).map_or(secs, |m| m.0);
        let kind = BuiltinDevice::Sampler {
            sample,
            slices: SliceSettings {
                enabled: false,
                ..slices.clone()
            },
        };
        // The sampler's sound (mode, envelope, volume...), playing just the slice at its
        // original pitch (pads always receive `PAD_PLAY_NOTE`).
        let mut p = builtin_params(&kind, &[]);
        p.extend(params.iter().map(|(k, v)| (*k, *v)));
        p.insert(sampler::params::ROOT_KEY, PAD_PLAY_NOTE as f64);
        p.insert(sampler::params::START, percent(start));
        p.insert(sampler::params::END, percent(end));
        ctx.tx.insert(Entity::Device(Device {
            sidechain_tap: None,
            chain: None,
            id: pid.device,
            track,
            order: pad_order.clone(),
            name: name.to_owned(),
            enabled: true,
            kind: DeviceKind::Builtin { device: kind },
            params: p,
            sidechain: None,
            pad: Some(pid.pad),
        }))?;
    }
    Ok(())
}

fn no_slices() -> ether_core::protocol::CommandError {
    crate::tx::keyed_err(
        ether_core::protocol::ErrorCode::InvalidState,
        crate::text::msg("error.drum_rack.no_slices"),
    )
}

/// `SliceCommand::ToDrumRack` (see its docs): one sampler pad per playable slice, the rack
/// in the sampler's place.
fn to_drum_rack(
    ctx: &mut DocCtx,
    device: DeviceId,
    rack_id: DeviceId,
    ids: &[SlicePadIds],
) -> CmdResult<()> {
    if ctx.p().devices.contains_key(&rack_id) && !ctx.p().devices.contains_key(&device) {
        // Retried after it succeeded.
        return Ok(());
    }
    let (d, sample, s) = sampler_of(ctx, device)?;
    if d.pad.is_some() || ctx.p().is_under_drum_pad(d.id) {
        return Err(invalid("drum racks cannot be nested in pads"));
    }
    if ctx.p().devices.contains_key(&rack_id) {
        return Err(invalid(format!("device {rack_id} already exists")));
    }
    let (_, secs) = sample_seconds(ctx, sample)?;
    let count = s.markers.len().min(128 - s.base_note as usize);
    if count == 0 {
        return Err(no_slices());
    }
    check_slice_ids(ctx, ids, count, rack_id)?;
    let rack_kind = BuiltinDevice::DrumRack;
    ctx.tx.insert(Entity::Device(Device {
        sidechain_tap: None,
        // In the sampler's place: on the track chain or a rack chain.
        chain: d.chain,
        id: rack_id,
        track: d.track,
        order: d.order.clone(),
        name: default_device_name(&rack_kind),
        enabled: true,
        params: builtin_params(&rack_kind, &[]),
        kind: DeviceKind::Builtin { device: rack_kind },
        sidechain: None,
        pad: None,
    }))?;
    add_slice_pads(
        ctx,
        rack_id,
        d.track,
        sample,
        secs,
        &s,
        &ids[..count],
        &d.name,
        &d.params,
    )?;
    ctx.delete_device(d.id)
}

/// `SliceCommand::SampleToPads` (see its docs): `media` sliced onto the pads of an
/// existing rack.
fn sample_to_pads(
    ctx: &mut DocCtx,
    rack_id: DeviceId,
    media: MediaId,
    mode: AutoSlice,
    base_note: u8,
    ids: &[SlicePadIds],
) -> CmdResult<()> {
    check_note(base_note)?;
    let r = rack(ctx, rack_id)?;
    if ids
        .first()
        .and_then(|i| ctx.p().drum_pads.get(&i.pad))
        .is_some_and(|p| p.rack == rack_id)
    {
        // Retried after it succeeded.
        return Ok(());
    }
    let (m, secs) = sample_seconds(ctx, Some(media))?;
    let markers = normalize(auto_markers(ctx, &m, secs, mode)?);
    let count = markers.len().min(128 - base_note as usize);
    if count == 0 {
        return Err(no_slices());
    }
    check_slice_ids(ctx, ids, count, rack_id)?;
    // The slices' notes are theirs: pads already there go, with their chains.
    let notes = base_note as usize..base_note as usize + count;
    let replaced: Vec<DrumPadId> = ctx
        .p()
        .pads_of(rack_id)
        .iter()
        .filter(|p| notes.contains(&(p.note as usize)))
        .map(|p| p.id)
        .collect();
    for id in replaced {
        ctx.delete_pad(id)?;
    }
    let slices = SliceSettings {
        enabled: false,
        base_note,
        markers,
    };
    let kind = BuiltinDevice::Sampler {
        sample: Some(media),
        slices: SliceSettings::default(),
    };
    add_slice_pads(
        ctx,
        rack_id,
        r.track,
        Some(media),
        secs,
        &slices,
        &ids[..count],
        &default_device_name(&kind),
        &BTreeMap::new(),
    )
}

/// Pad chains of the drum racks of `track`: on its chain, then the ones nested in rack chains
/// (by id; the engine runs their pads at their rack-chain entry). Racks and pad devices
/// without an engine node are skipped.
pub fn racks_desc(p: &Project, track: TrackId, ctx: &CompileContext) -> Vec<RackDesc> {
    let mut nested: Vec<&Device> = p
        .devices
        .values()
        .filter(|d| d.track == track && d.chain.is_some() && is_rack(d))
        .collect();
    nested.sort_by_key(|d| d.id);
    p.devices_of(track)
        .into_iter()
        .filter(|d| is_rack(d))
        .chain(nested)
        .filter_map(|r| {
            let rack = (ctx.nodes)(r.id)?;
            let pads = p
                .pads_of(r.id)
                .into_iter()
                .map(|pad| PadDesc {
                    pad: pad.id,
                    note: pad.note,
                    choke_group: pad.choke_group,
                    chain: p
                        .pad_devices_of(pad.id)
                        .into_iter()
                        .filter_map(|d| {
                            (ctx.nodes)(d.id).map(|node| ChainEntry {
                                node,
                                enabled: d.enabled,
                                sidechain: d.sidechain,
                            })
                        })
                        .collect(),
                    volume: pad.volume.to_linear(),
                    pan: pad.pan.0,
                    mute: pad.mute,
                })
                .collect();
            Some(RackDesc { rack, pads })
        })
        .collect()
}

/// Pad solo: in every rack with a soloed pad, the other pads compile as muted.
pub fn apply_solo(desc: &mut RenderGraphDesc, solo: &BTreeSet<DrumPadId>) {
    if solo.is_empty() {
        return;
    }
    for rack in desc.tracks.iter_mut().flat_map(|t| t.racks.iter_mut()) {
        if rack.pads.iter().any(|p| solo.contains(&p.pad)) {
            for p in &mut rack.pads {
                p.mute |= !solo.contains(&p.pad);
            }
        }
    }
}

/// Whether a built-in node built from `old` can take `new` in place
/// (`EngineBridge::update_builtin`): only a sampler's slices changed.
pub fn updatable_in_place(old: &BuiltinDevice, new: &BuiltinDevice) -> bool {
    matches!(
        (old, new),
        (
            BuiltinDevice::Sampler { sample: a, slices: sa },
            BuiltinDevice::Sampler { sample: b, slices: sb },
        ) if a == b && sa != sb
    )
}
