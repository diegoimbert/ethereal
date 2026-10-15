//! VST3 parameters → `ParamInfo`, and the plain ↔ normalized rule of
//! `docs/PLUGIN-FORMATS.md` (usable on any thread, unlike
//! `IEditController::normalizedParamToPlain`):
//!
//! - continuous (`stepCount == 0`): plain = normalized, `0..=1`;
//! - discrete (`stepCount = n > 0`): plain = step index `0..=n`, normalized = plain / n.

use ether_core::protocol::devices::{ParamInfo, ParamScale, ParamUnit};
use ether_core::protocol::model::ParamId;
use vst3::ComPtr;
use vst3::Steinberg::Vst::ParameterInfo_::ParameterFlags_::{kCanAutomate, kIsHidden, kIsReadOnly};
use vst3::Steinberg::Vst::{IEditController, IEditControllerTrait, ParameterInfo, String128};
use vst3::Steinberg::kResultOk;

use crate::host::read_tchar;

/// Largest step count for which labels are generated.
const MAX_LABELS: i32 = 128;

/// Plain value → normalized, for a param with `steps` (`stepCount`).
pub fn to_normalized(plain: f64, steps: u32) -> f64 {
    if steps == 0 {
        plain.clamp(0.0, 1.0)
    } else {
        (plain.round() / f64::from(steps)).clamp(0.0, 1.0)
    }
}

/// Normalized value → plain, for a param with `steps` (`stepCount`).
pub fn to_plain(normalized: f64, steps: u32) -> f64 {
    let n = normalized.clamp(0.0, 1.0);
    if steps == 0 {
        n
    } else {
        (n * f64::from(steps)).round()
    }
}

/// Param id → step count, sorted by id (binary-searchable on the audio thread).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StepTable(Vec<(u32, u32)>);

impl StepTable {
    pub fn new(mut entries: Vec<(u32, u32)>) -> Self {
        entries.sort_unstable();
        entries.dedup_by_key(|(id, _)| *id);
        Self(entries)
    }

    pub fn steps(&self, id: u32) -> Option<u32> {
        self.0
            .binary_search_by_key(&id, |(k, _)| *k)
            .ok()
            .map(|i| self.0[i].1)
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().map(|(id, _)| *id)
    }
}

/// Param id → its `units` string (`"dB"`), for the ones that have one.
pub(crate) type Units = std::collections::HashMap<u32, String>;

/// Query every parameter of `controller` (main thread).
pub(crate) fn list(controller: &ComPtr<IEditController>) -> (Vec<ParamInfo>, StepTable, Units) {
    // SAFETY (all calls): valid controller on its main thread; out structs are plain C data.
    let count = unsafe { controller.getParameterCount() }.max(0);
    let mut out = Vec::with_capacity(count as usize);
    let mut steps = Vec::with_capacity(count as usize);
    let mut units = Units::new();
    for index in 0..count {
        let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
        if unsafe { controller.getParameterInfo(index, &mut info) } != kResultOk {
            continue;
        }
        let id = info.id;
        let n = info.stepCount.max(0);
        let flags = info.flags as u32;
        let name = match read_tchar(&info.title) {
            t if t.is_empty() => read_tchar(&info.shortTitle),
            t => t,
        };
        let (max, default, labels) = if n == 0 {
            (1.0, info.defaultNormalizedValue.clamp(0.0, 1.0), None)
        } else {
            let labels = (n <= MAX_LABELS).then(|| {
                (0..=n)
                    .map(|i| {
                        let norm = f64::from(i) / f64::from(n);
                        let mut s: String128 = [0; 128];
                        let ok = unsafe { controller.getParamStringByValue(id, norm, &mut s) }
                            == kResultOk;
                        match read_tchar(&s) {
                            t if ok && !t.is_empty() => t,
                            _ => i.to_string(),
                        }
                    })
                    .collect()
            });
            (
                f64::from(n),
                to_plain(info.defaultNormalizedValue, n as u32),
                labels,
            )
        };
        steps.push((id, n as u32));
        let unit = read_tchar(&info.units);
        if !unit.trim().is_empty() {
            units.insert(id, unit.trim().to_owned());
        }
        out.push(ParamInfo {
            step: None,
            remote: None,
            id: ParamId(id),
            name,
            group: None,
            unit: if n == 1 {
                ParamUnit::Toggle
            } else {
                ParamUnit::None
            },
            min: 0.0,
            max,
            default,
            scale: ParamScale::Linear,
            labels,
            automatable: flags & kCanAutomate as u32 != 0 && flags & kIsReadOnly as u32 == 0,
            hidden: flags & kIsHidden as u32 != 0,
        });
    }
    (out, StepTable::new(steps), units)
}

/// The plugin's text for `normalized` (`getParamStringByValue`) with the param's units
/// appended unless the text already ends with them (`"-6.0 dB"`). Main thread.
pub(crate) fn text(
    controller: &ComPtr<IEditController>,
    id: u32,
    normalized: f64,
    units: Option<&str>,
) -> Option<String> {
    let mut s: String128 = [0; 128];
    // SAFETY: valid controller on its main thread; `s` is a String128.
    if unsafe { controller.getParamStringByValue(id, normalized, &mut s) } != kResultOk {
        return None;
    }
    with_units(&read_tchar(&s), units)
}

/// `text` + `" " + units` (trimmed), unless `text` is empty or already ends with the units.
pub(crate) fn with_units(text: &str, units: Option<&str>) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(match units.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) if !text.ends_with(u) => format!("{text} {u}"),
        _ => text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_appended_once() {
        assert_eq!(with_units("-6.0", Some("dB")).as_deref(), Some("-6.0 dB"));
        assert_eq!(
            with_units("-6.0 dB", Some("dB")).as_deref(),
            Some("-6.0 dB")
        );
        assert_eq!(with_units(" Saw ", None).as_deref(), Some("Saw"));
        assert_eq!(with_units("1.0", Some(" ")).as_deref(), Some("1.0"));
        assert_eq!(with_units("  ", Some("dB")), None);
    }

    #[test]
    fn plain_normalized_rule() {
        assert_eq!(to_normalized(0.25, 0), 0.25);
        assert_eq!(to_plain(0.25, 0), 0.25);
        assert_eq!(to_normalized(2.0, 0), 1.0);
        assert_eq!(to_normalized(1.0, 4), 0.25);
        assert_eq!(to_plain(0.25, 4), 1.0);
        assert_eq!(to_plain(0.3, 4), 1.0);
        assert_eq!(to_plain(0.4, 4), 2.0);
        assert_eq!(to_normalized(7.0, 4), 1.0);
        for n in 1..10u32 {
            for step in 0..=n {
                let p = f64::from(step);
                assert_eq!(to_plain(to_normalized(p, n), n), p);
            }
        }
        let t = StepTable::new(vec![(9, 1), (2, 0), (5, 3)]);
        assert_eq!(t.steps(5), Some(3));
        assert_eq!(t.steps(2), Some(0));
        assert_eq!(t.steps(3), None);
        assert_eq!(t.ids().collect::<Vec<_>>(), [2, 5, 9]);
    }
}
