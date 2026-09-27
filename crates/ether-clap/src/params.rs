//! CLAP parameter metadata → `ParamInfo`.
//!
//! CLAP values are already *plain* values in `[min, max]`, which is exactly what the document
//! stores, so ids and values pass through unchanged and the scale is `Linear`.

use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags, PluginParams};
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
