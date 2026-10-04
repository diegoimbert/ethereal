//! Scanning a VST2 library: open its plugin (or, for a shell, each sub-plugin) and describe
//! it. Runs ONLY inside the `ether-plugin-scanner` process (it loads plugin code).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ether_core::plugin::PluginError;
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;

use crate::abi::*;
use crate::module::{Effect, Library, LoadError};

/// Most sub-plugins of one shell that are listed.
const MAX_SHELL_PLUGINS: usize = 4096;
/// Time budget for opening the sub-plugins of a shell one by one (category, vendor, MIDI
/// input). Past it the remaining ones are listed from the shell's name list only, so a big
/// shell stays well inside the scanner's 20 s timeout.
const SHELL_PROBE_BUDGET: Duration = Duration::from_secs(12);

/// What the scan and the controller know about a plugin.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Info {
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub features: Vec<String>,
    pub category: DeviceCategory,
    pub midi_input: bool,
}

/// Feature tags for a plugin category (`effGetPlugCategory`), plus `instrument` for synths.
pub fn features_for(category: isize, is_synth: bool) -> Vec<String> {
    let mut f: Vec<&str> = match category {
        kPlugCategSynth => vec!["instrument", "synth"],
        kPlugCategAnalysis => vec!["fx", "analyzer"],
        kPlugCategMastering => vec!["fx", "mastering"],
        kPlugCategSpacializer => vec!["fx", "spatial"],
        kPlugCategRoomFx => vec!["fx", "reverb"],
        kPlugSurroundFx => vec!["fx", "surround"],
        kPlugCategRestoration => vec!["fx", "restoration"],
        kPlugCategGenerator => vec!["instrument", "generator"],
        _ if is_synth => vec![],
        _ => vec!["fx"],
    };
    if is_synth && !f.contains(&"instrument") {
        f.insert(0, "instrument");
    }
    f.into_iter().map(str::to_owned).collect()
}

/// Instrument when the plugin is a synth (`effFlagsIsSynth`, or the synth/generator
/// category); an audio effect otherwise.
pub fn category_from_features(features: &[String]) -> DeviceCategory {
    if features.iter().any(|f| f == "instrument") {
        DeviceCategory::Instrument
    } else {
        DeviceCategory::AudioEffect
    }
}

/// Describe an open effect. `shell_name` is the name a shell gave the sub-plugin.
pub(crate) fn info(effect: &Effect, path: &Path, shell_name: Option<&str>) -> Info {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = [
        shell_name.map(str::to_owned).unwrap_or_default(),
        effect.string(effGetEffectName, 0),
        effect.string(effGetProductString, 0),
        stem,
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or_default();
    let vendor = effect.string(effGetVendorString, 0);
    let version = match effect.dispatch(effGetVendorVersion, 0, 0, std::ptr::null_mut(), 0.0) {
        v if v > 0 => v.to_string(),
        _ if effect.version() > 0 => effect.version().to_string(),
        _ => String::new(),
    };
    let is_synth = effect.has_flag(effFlagsIsSynth);
    let features = features_for(effect.category(), is_synth);
    let category = category_from_features(&features);
    let midi_input = category == DeviceCategory::Instrument
        || effect.can_do(c"receiveVstMidiEvent") > 0
        || effect.can_do(c"receiveVstEvents") > 0;
    Info {
        name,
        vendor,
        version,
        features,
        category,
        midi_input,
    }
}

fn descriptor(id: i32, info: Info, path: &Path) -> PluginDescriptor {
    PluginDescriptor {
        sidechain_inputs: 0,
        format: PluginFormat::Vst2,
        id: crate::plugin_id(id),
        name: info.name,
        vendor: info.vendor,
        version: info.version,
        description: String::new(),
        features: info.features,
        category: info.category,
        path: path.to_string_lossy().into_owned(),
    }
}

/// The sub-plugins of an open shell (`effShellGetNextPlugin`): `(unique id, name)`.
fn shell_plugins(shell: &Effect) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    while out.len() < MAX_SHELL_PLUGINS {
        let mut buf = [0u8; STRING_BUF];
        let id = shell.dispatch(effShellGetNextPlugin, 0, 0, buf.as_mut_ptr().cast(), 0.0);
        if id == 0 {
            break;
        }
        buf[STRING_BUF - 1] = 0;
        out.push((id as i32, read_cstr(&buf)));
    }
    out
}

/// Load one library and describe its plugins. A library that isn't a VST2 plugin (no entry
/// point: VST2 folders often hold helper libraries) yields no plugins rather than an error.
/// Called ONLY inside the scanner process.
pub fn scan_library(path: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
    let lib = match Library::load(path) {
        Ok(lib) => Arc::new(lib),
        Err(LoadError::NotVst2) => return Ok(Vec::new()),
        Err(LoadError::Plugin(e)) => return Err(e),
    };
    let effect = Effect::create(lib.clone(), 0)?;
    if effect.category() != kPlugCategShell {
        let info = info(&effect, path, None);
        return Ok(vec![descriptor(effect.unique_id(), info, path)]);
    }
    // A shell (Waves-style): one library, many plugins chosen by `audioMasterCurrentId`.
    let subs = shell_plugins(&effect);
    let shell_vendor = effect.string(effGetVendorString, 0);
    drop(effect);
    let deadline = Instant::now() + SHELL_PROBE_BUDGET;
    let mut out = Vec::with_capacity(subs.len());
    for (id, name) in subs {
        let probed = (Instant::now() < deadline)
            .then(|| Effect::create(lib.clone(), id).ok())
            .flatten()
            .filter(|e| e.category() != kPlugCategShell)
            .map(|e| info(&e, path, Some(&name)));
        let info = probed.unwrap_or_else(|| Info {
            features: features_for(kPlugCategEffect, false),
            category: DeviceCategory::AudioEffect,
            name,
            vendor: shell_vendor.clone(),
            version: String::new(),
            midi_input: false,
        });
        out.push(descriptor(id, info, path));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_and_features() {
        let f = features_for(kPlugCategEffect, false);
        assert_eq!(f, ["fx"]);
        assert_eq!(category_from_features(&f), DeviceCategory::AudioEffect);
        let f = features_for(kPlugCategSynth, true);
        assert_eq!(f, ["instrument", "synth"]);
        assert_eq!(category_from_features(&f), DeviceCategory::Instrument);
        // The synth flag wins over a missing/odd category.
        let f = features_for(kPlugCategUnknown, true);
        assert_eq!(f, ["instrument"]);
        let f = features_for(kPlugCategRoomFx, true);
        assert_eq!(f, ["instrument", "fx", "reverb"]);
        assert_eq!(features_for(kPlugCategRoomFx, false), ["fx", "reverb"]);
        assert_eq!(
            category_from_features(&features_for(kPlugCategGenerator, false)),
            DeviceCategory::Instrument
        );
    }
}
