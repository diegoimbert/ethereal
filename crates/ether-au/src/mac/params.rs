//! `AUParameterTree` → `ParamInfo`, with stable ids ([`crate::ids::ParamMap`]).
//!
//! AU values are already plain (`minValue..maxValue` in the parameter's unit), so they pass
//! through unchanged; the scale is `Log` for `kAudioUnitParameterFlag_DisplayLogarithmic`
//! (when `min > 0`), `Linear` otherwise. `AUParameter` has no default value, so the default
//! is the value the parameter had when this instance first saw it (fresh instance = the
//! unit's default).

use std::collections::HashMap;

use ether_core::protocol::devices::{ParamInfo, ParamScale, ParamUnit};
use ether_core::protocol::model::ParamId;
use objc2::rc::Retained;
use objc2_audio_toolbox::{
    AUAudioUnit, AUParameter, AUParameterTree, AudioUnitParameterOptions, AudioUnitParameterUnit,
};

use crate::ids::ParamMap;

/// Largest indexed range for which labels are generated.
const MAX_LABELS: usize = 256;

pub(crate) struct ParamSet {
    pub tree: Option<Retained<AUParameterTree>>,
    pub map: ParamMap,
    pub infos: Vec<ParamInfo>,
    /// `(id, parameter)` sorted by id.
    pub objects: Vec<(u32, Retained<AUParameter>)>,
    /// First-seen value per address (the default).
    defaults: HashMap<u64, f64>,
}

impl ParamSet {
    pub fn empty() -> Self {
        Self {
            tree: None,
            map: ParamMap::default(),
            infos: Vec::new(),
            objects: Vec::new(),
            defaults: HashMap::new(),
        }
    }

    /// (Re)read the unit's parameter tree. `saved` = the id table from a loaded state
    /// (else the current mapping is kept for addresses that still exist).
    pub fn refresh(&mut self, au: &AUAudioUnit, saved: Option<&[(u64, u32)]>) {
        // SAFETY: plain property getters on the main thread.
        let tree = unsafe { au.parameterTree() };
        let params: Vec<Retained<AUParameter>> = match &tree {
            Some(t) => unsafe { t.allParameters() }.to_vec(),
            None => Vec::new(),
        };
        let addrs: Vec<u64> = params.iter().map(|p| unsafe { p.address() }).collect();
        let keep: Vec<(u64, u32)>;
        let saved = match saved {
            Some(s) => s,
            None => {
                keep = self.map.entries().to_vec();
                &keep
            }
        };
        self.map = ParamMap::build(&addrs, saved);
        self.infos = Vec::with_capacity(params.len());
        self.objects = Vec::with_capacity(params.len());
        for p in params {
            let addr = unsafe { p.address() };
            let Some(id) = self.map.id(addr) else {
                continue;
            };
            let value = f64::from(unsafe { p.value() });
            let default = *self.defaults.entry(addr).or_insert(value);
            self.infos.push(info(&p, id, default));
            self.objects.push((id, p));
        }
        self.objects.sort_by_key(|(id, _)| *id);
        self.tree = tree;
    }

    pub fn object(&self, id: u32) -> Option<&AUParameter> {
        self.objects
            .binary_search_by_key(&id, |(i, _)| *i)
            .ok()
            .map(|i| &*self.objects[i].1)
    }
}

fn unit(u: AudioUnitParameterUnit) -> ParamUnit {
    match u {
        AudioUnitParameterUnit::Decibels => ParamUnit::Decibels,
        AudioUnitParameterUnit::Hertz => ParamUnit::Hertz,
        AudioUnitParameterUnit::Milliseconds => ParamUnit::Milliseconds,
        AudioUnitParameterUnit::Seconds => ParamUnit::Seconds,
        AudioUnitParameterUnit::Percent => ParamUnit::Percent,
        AudioUnitParameterUnit::RelativeSemiTones => ParamUnit::Semitones,
        AudioUnitParameterUnit::Ratio => ParamUnit::Ratio,
        AudioUnitParameterUnit::Pan => ParamUnit::Pan,
        AudioUnitParameterUnit::Boolean => ParamUnit::Toggle,
        _ => ParamUnit::None,
    }
}

fn info(p: &AUParameter, id: u32, default: f64) -> ParamInfo {
    // SAFETY: plain property getters.
    let (name, min, max, flags, u) = unsafe {
        (
            p.displayName().to_string(),
            f64::from(p.minValue()),
            f64::from(p.maxValue()),
            p.flags(),
            p.unit(),
        )
    };
    let display = flags.0 & AudioUnitParameterOptions::Flag_DisplayMask.0;
    let scale = if display == AudioUnitParameterOptions::Flag_DisplayLogarithmic.0 && min > 0.0 {
        ParamScale::Log
    } else {
        ParamScale::Linear
    };
    let labels = if u == AudioUnitParameterUnit::Indexed {
        unsafe { p.valueStrings() }
            .map(|v| v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .filter(|l| {
                !l.is_empty()
                    && l.len() <= MAX_LABELS
                    && min.fract() == 0.0
                    && (max - min) as usize + 1 == l.len()
            })
    } else {
        None
    };
    let writable = flags.contains(AudioUnitParameterOptions::Flag_IsWritable);
    let meter = flags.contains(AudioUnitParameterOptions::Flag_MeterReadOnly);
    ParamInfo {
        step: None,
        remote: None,
        id: ParamId(id),
        name,
        group: None,
        unit: unit(u),
        min,
        max,
        default: default.clamp(min.min(max), max.max(min)),
        scale,
        labels,
        automatable: writable && !meter,
        hidden: meter || !writable,
    }
}
