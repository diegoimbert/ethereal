//! CLAP parameter metadata → `ParamInfo`.
//!
//! CLAP values are already *plain* values in `[min, max]`, which is exactly what the document
//! stores, so ids and values pass through unchanged and the scale is `Linear`.

use std::collections::HashMap;

use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags, PluginParams};
use clack_extensions::remote_controls::{PluginRemoteControls, RemoteControlsPageBuffer};
use clack_host::prelude::*;
use ether_core::protocol::devices::{ParamInfo, ParamScale, ParamUnit};
use ether_core::protocol::model::ParamId;

/// Largest stepped range for which labels are generated.
const MAX_LABELS: f64 = 128.0;

pub(crate) fn list(ext: &PluginParams, plugin: &PluginMainThreadHandle) -> Vec<ParamInfo> {
    let count = ext.count(plugin);
    let mut out = Vec::with_capacity(count as usize);
    let mut buffer = ParamInfoBuffer::new();
    for index in 0..count {
        let Some(info) = ext.get_info(plugin, index, &mut buffer) else {
            continue;
        };
        let id = info.id.get();
        let flags = info.flags;
        let name = String::from_utf8_lossy(info.name).into_owned();
        let module = String::from_utf8_lossy(info.module).into_owned();
        let (min, max, default) = (info.min_value, info.max_value, info.default_value);

        let stepped = flags.intersects(ParamInfoFlags::IS_STEPPED | ParamInfoFlags::IS_ENUM);
        let labels = if stepped
            && max > min
            && max - min <= MAX_LABELS
            && min.fract() == 0.0
            && max.fract() == 0.0
        {
            let steps = (max - min) as u32;
            Some(
                (0..=steps)
                    .map(|i| {
                        let v = min + f64::from(i);
                        value_text(ext, plugin, id, v).unwrap_or_else(|| format!("{v}"))
                    })
                    .collect(),
            )
        } else {
            None
        };
        let unit = if stepped && min == 0.0 && max == 1.0 {
            ParamUnit::Toggle
        } else {
            ParamUnit::None
        };

        out.push(ParamInfo {
            step: None,
            remote: None,
            id: ParamId(id),
            name,
            group: (!module.is_empty()).then_some(module),
            unit,
            min,
            max,
            default,
            scale: ParamScale::Linear,
            labels,
            automatable: flags.contains(ParamInfoFlags::IS_AUTOMATABLE)
                && !flags.contains(ParamInfoFlags::IS_READONLY),
            hidden: flags.contains(ParamInfoFlags::IS_HIDDEN),
        });
    }
    out
}

fn value_text(
    ext: &PluginParams,
    plugin: &PluginMainThreadHandle,
    id: u32,
    value: f64,
) -> Option<String> {
    let mut buf = [0u8; 256];
    let text = ext
        .value_to_text(plugin, ClapId::new(id), value, &mut buf)
        .ok()?;
    let s = String::from_utf8_lossy(text).trim().to_owned();
    (!s.is_empty()).then_some(s)
}

/// Max remote-control pages read (8 controls each); the device card shows at most a few.
const MAX_REMOTE_PAGES: u32 = 64;

/// The plugin's remote-control pages (`clap.remote-controls`), flattened in page order:
/// param id → `page index · 8 + slot` of its first occurrence.
pub(crate) fn remote_slots(
    ext: &PluginRemoteControls,
    plugin: &PluginMainThreadHandle,
) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    let mut buffer = RemoteControlsPageBuffer::new();
    for page in 0..ext.count(plugin).min(MAX_REMOTE_PAGES) {
        let Some(p) = ext.get(plugin, page, &mut buffer) else {
            continue;
        };
        for (slot, id) in p.param_ids.iter().enumerate() {
            if let Some(id) = id {
                out.entry(id.get()).or_insert(page * 8 + slot as u32);
            }
        }
    }
    out
}

/// Set `ParamInfo::remote` from [`remote_slots`].
pub(crate) fn apply_remote_slots(params: &mut [ParamInfo], slots: &HashMap<u32, u32>) {
    for p in params {
        p.remote = slots.get(&p.id.0).copied();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_slots_mark_params() {
        let info = |id| ParamInfo {
            id: ParamId(id),
            name: format!("p{id}"),
            group: None,
            unit: ParamUnit::None,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            scale: ParamScale::Linear,
            labels: None,
            automatable: true,
            hidden: false,
            step: None,
            remote: None,
        };
        let mut params = vec![info(1), info(2), info(3)];
        apply_remote_slots(&mut params, &HashMap::from([(3, 0), (1, 9)]));
        assert_eq!(
            params.iter().map(|p| p.remote).collect::<Vec<_>>(),
            vec![Some(9), None, Some(0)]
        );
    }
}
