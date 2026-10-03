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

/// Query every parameter of `controller` (main thread).
pub(crate) fn list(controller: &ComPtr<IEditController>) -> (Vec<ParamInfo>, StepTable) {
    // SAFETY (all calls): valid controller on its main thread; out structs are plain C data.
    let count = unsafe { controller.getParameterCount() }.max(0);
    let mut out = Vec::with_capacity(count as usize);
    let mut steps = Vec::with_capacity(count as usize);
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
    (out, StepTable::new(steps))
}

#[cfg(test)]
mod tests {
    use super::*;

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
