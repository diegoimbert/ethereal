//! Racks, macros and modulation (v0.2, `racks-modulation` node; CONTRACTS.md §12.6). Data
//! model: `ether_model::{rack, modulation}`. All commands are undoable document commands
//! except `ListModulatorKinds`.
//!
//! Rack devices are inserted with `DeviceCommand::Insert` like any built-in
//! (`BuiltinDevice::{InstrumentRack, AudioEffectRack, MidiEffectRack}`); chain devices are
//! ordinary devices (params, rename, bypass, remove, automation by device id) and only
//! inserting into / moving between chains needs these commands. Macros are the rack's params
//! `0..8` (`DeviceCommand::SetParam`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::devices::{DeviceSpec, ParamInfo};
use crate::model::{
    BuiltinDeviceType, Color, Decibels, DeviceId, ModMappingId, ModSource, ModulatorId,
    ModulatorKind, Pan, ParamId, RackChainId, Zone,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum RackCommand {
    /// New empty chain (full zones, unity gain) before `before` (None = last).
    AddChain {
        id: RackChainId,
        rack: DeviceId,
        name: Option<String>,
        before: Option<RackChainId>,
    },
    /// Removes the chain with its devices (and their modulators/mappings).
    RemoveChain {
        id: RackChainId,
    },
    RenameChain {
        id: RackChainId,
        name: String,
    },
    SetChainColor {
        id: RackChainId,
        color: Option<Color>,
    },
    MoveChain {
        id: RackChainId,
        before: Option<RackChainId>,
    },
    /// Partial update (`None` = unchanged). Continuous: send with a gesture.
    SetChainMix {
        id: RackChainId,
        volume: Option<Decibels>,
        pan: Option<Pan>,
        mute: Option<bool>,
        solo: Option<bool>,
    },
    /// Partial update of the key / velocity / chain-selector zones.
    SetChainZones {
        id: RackChainId,
        keys: Option<Zone>,
        velocities: Option<Zone>,
        select: Option<Zone>,
    },
    /// Insert a device on a chain before `before` (None = end).
    InsertDevice {
        id: DeviceId,
        chain: RackChainId,
        device: DeviceSpec,
        before: Option<DeviceId>,
    },
    /// Move a device (same track) onto a chain (`chain: Some`) or back to the track chain
    /// (`chain: None`, before `before` among track devices).
    MoveDevice {
        id: DeviceId,
        chain: Option<RackChainId>,
        before: Option<DeviceId>,
    },
    /// Wrap consecutive track-chain devices into a new rack of `rack_type` with one chain
    /// `chain` holding them (Ableton "Group"). The rack takes the first device's position.
    Group {
        rack: DeviceId,
        rack_type: BuiltinDeviceType,
        chain: RackChainId,
        devices: Vec<DeviceId>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ModulationCommand {
    /// Add a modulator inside `device` (default params). `name: None` = the kind's name.
    AddModulator {
        id: ModulatorId,
        device: DeviceId,
        kind: ModulatorKind,
        name: Option<String>,
    },
    /// Removes the modulator and its mappings.
    RemoveModulator {
        id: ModulatorId,
    },
    RenameModulator {
        id: ModulatorId,
        name: String,
    },
    /// Plain value (clamped to the kind's param range). Continuous: send with a gesture.
    SetModulatorParam {
        modulator: ModulatorId,
        param: ParamId,
        value: f64,
    },
    ResetModulatorParam {
        modulator: ModulatorId,
        param: ParamId,
    },
    /// Map a source to a device param (drag from the modulator/macro onto a knob).
    /// `InvalidArgument` when out of scope (see `ether_model::modulation`) or duplicate.
    Map {
        id: ModMappingId,
        source: ModSource,
        device: DeviceId,
        param: ParamId,
        depth: f64,
    },
    /// `-1..=1`. Continuous: send with a gesture.
    SetDepth {
        id: ModMappingId,
        depth: f64,
    },
    Unmap {
        id: ModMappingId,
    },
    /// Replies `ModulatorKinds`.
    ListModulatorKinds,
}

/// Static description of a modulator kind (param table frozen in
/// `ether_devices::modulators`, append-only ids).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ModulatorDescriptor {
    pub kind: ModulatorKind,
    pub name: String,
    /// Source range `-1..=1` (else `0..=1`).
    pub bipolar: bool,
    pub params: Vec<ParamInfo>,
}
