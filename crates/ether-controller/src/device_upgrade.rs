//! Old documents' built-in device params, rewritten on load where a device replaced a param
//! by others (the device still plays the old param, but the panel shows the new ones).
//!
//! - Auto Filter: `Type (legacy)` (low/high-pass 12 and 24 in one list) → `Type` + `Slope`.
//!   An automation lane or modulation of the legacy param moves to `Type`, its points
//!   remapped (a lane switching between the 12 and 24 dB variants keeps the type only).

use ether_core::protocol::model::{BuiltinDeviceType, DeviceId, ParamId};
use ether_devices::fx_color::auto_filter as af;
use ether_devices::fx_color::auto_filter_device::FilterType;
use ether_model::{AutomationTarget, DeviceKind, Project};

/// Rewrites `project` in place; returns whether anything changed.
pub fn upgrade_project(project: &mut Project) -> bool {
    let mut moved: Vec<DeviceId> = Vec::new();
    for (id, d) in &mut project.devices {
        let DeviceKind::Builtin { device } = &d.kind else {
            continue;
        };
        if device.device_type() == BuiltinDeviceType::AutoFilter
            && FilterType::upgrade_params(&mut d.params).is_some()
        {
            moved.push(*id);
        }
    }
    // Automation and modulation of `Type (legacy)` on any Auto Filter (also one whose
    // static value was already the default).
    let auto_filters: Vec<DeviceId> = project
        .devices
        .iter()
        .filter(|(_, d)| {
            matches!(&d.kind, DeviceKind::Builtin { device }
                if device.device_type() == BuiltinDeviceType::AutoFilter)
        })
        .map(|(id, _)| *id)
        .collect();
    if auto_filters.is_empty() {
        return false;
    }
    let desc = ether_devices::descriptor(BuiltinDeviceType::AutoFilter);
    let info = |id: ParamId| desc.params.iter().find(|p| p.id == id);
    let (Some(legacy), Some(mode)) = (info(af::TYPE), info(af::MODE)) else {
        return !moved.is_empty();
    };
    let mut changed = !moved.is_empty();
    let mut lanes = Vec::new();
    for lane in project.automation_lanes.values_mut() {
        if let AutomationTarget::DeviceParam { device, param } = &mut lane.target
            && *param == af::TYPE
            && auto_filters.contains(device)
        {
            *param = af::MODE;
            lanes.push(lane.id);
        }
    }
    for p in project.automation_points.values_mut() {
        if lanes.contains(&p.lane) {
            let l = legacy.to_plain(p.value).round().max(0.0) as usize;
            p.value = mode.to_normalized(FilterType::legacy_mode(l) as f64);
        }
    }
    changed |= !lanes.is_empty();
    for m in project.mod_mappings.values_mut() {
        if m.param == af::TYPE && auto_filters.contains(&m.device) {
            m.param = af::MODE;
            changed = true;
        }
    }
    changed
}
