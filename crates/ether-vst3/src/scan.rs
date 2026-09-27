//! Scanning a `.vst3` bundle: list its audio-module classes as `PluginDescriptor`s.
//! Runs ONLY inside the `ether-plugin-scanner` process (it loads plugin code).

use std::collections::HashMap;
use std::path::Path;

use ether_core::plugin::PluginError;
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::PluginDescriptor;
use vst3::Steinberg::{
    IPluginFactory2, IPluginFactory2Trait, IPluginFactoryTrait, PClassInfo, PClassInfo2,
    PFactoryInfo, kResultOk,
};

use crate::class_id_to_string;
use crate::host::read_char8;
use crate::module::Module;

/// `PClassInfo::category` of processor classes (`kVstAudioEffectClass`).
pub const AUDIO_MODULE_CLASS: &str = "Audio Module Class";

/// One class of a factory.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ClassInfo {
    pub cid: [u8; 16],
    pub category: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// `|`-separated sub-categories (`Fx|Delay`, `Instrument|Synth`).
    pub sub_categories: String,
}

impl ClassInfo {
    pub fn id(&self) -> String {
        class_id_to_string(&self.cid)
    }

    pub fn features(&self) -> Vec<String> {
        features_from_sub_categories(&self.sub_categories)
    }
}

/// Split VST3 sub-categories on `|`, lowercased (`Fx|Delay` → `["fx", "delay"]`).
pub fn features_from_sub_categories(sub: &str) -> Vec<String> {
    sub.split('|')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `Instrument` (incl. `Instrument|Synth`...) → Instrument; everything else (`Fx`, ...) is an
/// audio effect.
pub fn category_from_features(features: &[String]) -> DeviceCategory {
    if features.iter().any(|f| f == "instrument") {
        DeviceCategory::Instrument
    } else {
        DeviceCategory::AudioEffect
    }
}

/// Every class of the module's factory (IPluginFactory2 info when available).
pub(crate) fn classes(module: &Module) -> Vec<ClassInfo> {
    let factory = module.factory();
    // SAFETY (all calls): valid factory; out structs are plain C data.
    let mut info: PFactoryInfo = unsafe { std::mem::zeroed() };
    let factory_vendor = if unsafe { factory.getFactoryInfo(&mut info) } == kResultOk {
        read_char8(&info.vendor)
    } else {
        String::new()
    };
    let f2 = factory.cast::<IPluginFactory2>();
    let count = unsafe { factory.countClasses() }.max(0);
    let mut out = Vec::new();
    for i in 0..count {
        let class = match &f2 {
            Some(f2) => {
                let mut c: PClassInfo2 = unsafe { std::mem::zeroed() };
                (unsafe { f2.getClassInfo2(i, &mut c) } == kResultOk).then(|| ClassInfo {
                    cid: c.cid.map(|b| b as u8),
                    category: read_char8(&c.category),
                    name: read_char8(&c.name),
                    vendor: read_char8(&c.vendor),
                    version: read_char8(&c.version),
                    sub_categories: read_char8(&c.subCategories),
                })
            }
            None => None,
        };
        let class = class.or_else(|| {
            let mut c: PClassInfo = unsafe { std::mem::zeroed() };
            (unsafe { factory.getClassInfo(i, &mut c) } == kResultOk).then(|| ClassInfo {
                cid: c.cid.map(|b| b as u8),
                category: read_char8(&c.category),
                name: read_char8(&c.name),
                ..ClassInfo::default()
            })
        });
        if let Some(mut class) = class {
            if class.vendor.is_empty() {
                class.vendor = factory_vendor.clone();
            }
            out.push(class);
        }
    }
    out
}

/// Metadata from `Contents/Resources/moduleinfo.json` (VST 3.7.5+), by class id. Used to fill
/// in what an `IPluginFactory` (v1) cannot report (sub-categories, version, vendor).
fn module_info(bundle: &Path) -> HashMap<String, ClassInfo> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(bundle.join("Contents/Resources/moduleinfo.json"))
    else {
        return out;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&strip_json5(&text)) else {
        return out;
    };
    let factory_vendor = v["Factory Info"]["Vendor"].as_str().unwrap_or_default();
    for c in v["Classes"].as_array().into_iter().flatten() {
        let s = |k: &str| c[k].as_str().unwrap_or_default().to_owned();
        let Some(cid) = c["CID"].as_str().and_then(crate::parse_class_id) else {
            continue;
        };
        let sub = c["Sub Categories"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            })
            .unwrap_or_default();
        let vendor = match s("Vendor") {
            v if v.is_empty() => factory_vendor.to_owned(),
            v => v,
        };
        let info = ClassInfo {
            cid,
            category: s("Category"),
            name: s("Name"),
            vendor,
            version: s("Version"),
            sub_categories: sub,
        };
        out.insert(info.id(), info);
    }
    out
}

/// moduleinfo.json is JSON5-flavored: drop `//` line comments and trailing commas.
fn strip_json5(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ',' => {
                let rest: String = chars.clone().collect();
                let next = rest.trim_start().chars().next();
                if !matches!(next, Some(']') | Some('}')) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Load one bundle and list its audio-module classes. Called ONLY inside the scanner process.
pub fn scan_bundle(bundle: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
    let module = Module::load(bundle)?;
    let extra = module_info(bundle);
    let plugins = classes(&module)
        .into_iter()
        .filter(|c| c.category == AUDIO_MODULE_CLASS)
        .map(|mut c| {
            if let Some(e) = extra.get(&c.id()) {
                for (dst, src) in [
                    (&mut c.sub_categories, &e.sub_categories),
                    (&mut c.version, &e.version),
                    (&mut c.vendor, &e.vendor),
                    (&mut c.name, &e.name),
                ] {
                    if dst.is_empty() {
                        dst.clone_from(src);
                    }
                }
            }
            let features = c.features();
            PluginDescriptor {
                format: PluginFormat::Vst3,
                id: c.id(),
                name: c.name,
                vendor: c.vendor,
                version: c.version,
                description: String::new(),
                category: category_from_features(&features),
                features,
                path: bundle.to_string_lossy().into_owned(),
            }
        })
        .collect();
    Ok(plugins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        let f = features_from_sub_categories("Instrument|Synth");
        assert_eq!(f, ["instrument", "synth"]);
        assert_eq!(category_from_features(&f), DeviceCategory::Instrument);
        let f = features_from_sub_categories("Fx|Delay| ");
        assert_eq!(f, ["fx", "delay"]);
        assert_eq!(category_from_features(&f), DeviceCategory::AudioEffect);
        assert_eq!(category_from_features(&[]), DeviceCategory::AudioEffect);
    }

    #[test]
    fn reads_json5_module_info() {
        let dir = crate::testing::temp_dir("moduleinfo");
        let res = dir.join("X.vst3/Contents/Resources");
        std::fs::create_dir_all(&res).unwrap();
        std::fs::write(
            res.join("moduleinfo.json"),
            r#"{
  // comment, with "quotes"
  "Name": "X",
  "Factory Info": { "Vendor": "ACME", },
  "Classes": [
    {
      "CID": "0123456789ABCDEF0123456789ABCDEF",
      "Category": "Audio Module Class",
      "Name": "Thing // not a comment",
      "Sub Categories": ["Fx", "Delay",],
      "Version": "1.0",
    },
  ],
}"#,
        )
        .unwrap();
        let m = module_info(&dir.join("X.vst3"));
        let c = &m["0123456789ABCDEF0123456789ABCDEF"];
        assert_eq!(c.name, "Thing // not a comment");
        assert_eq!(c.vendor, "ACME");
        assert_eq!(c.sub_categories, "Fx|Delay");
        assert_eq!(c.version, "1.0");
        let _ = std::fs::remove_dir_all(dir);
    }
}
