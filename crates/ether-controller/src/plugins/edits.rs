//! Plugin-GUI param edits (perf-plugins, audit #6).
//!
//! A plugin reports every knob move of its own GUI as `ParamEdited`; a preset switch may
//! report every param at once, and many plugins never wrap edits in begin/end gestures.
//! Applied one by one, each event was a transaction with its own patch, collab message
//! and (without a gesture) undo step, so one preset change could flush the whole history.
//!
//! Per poll (one controller tick):
//! - edits of one `(device, param)` coalesce, last value wins; an edit to the value the
//!   document already has is dropped;
//! - other notifications keep their place: edits before a `GestureBegin`/`GestureEnd`
//!   (or anything else) are applied before it;
//! - edits inside a plugin gesture merge into that gesture's undo step as before;
//! - gesture-less edits of one device are one transaction, and gesture-less edits less
//!   than [`PLUGIN_EDIT_WINDOW_MS`] apart (any device) merge into one "plugin edit" undo
//!   step. Any other edit closes the step (`History` gestures).

use ether_core::plugin::PluginNotification;
use ether_core::protocol::model::{DeviceChange, DeviceId, GestureId, ParamId};

use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Gesture-less plugin edits closer together than this share one undo step.
pub(crate) const PLUGIN_EDIT_WINDOW_MS: u64 = 250;

/// Edits applied in one transaction: a device and its plugin gesture (if any).
type EditGroup = (DeviceId, Option<GestureId>);

/// Undo label (message id) of grouped gesture-less plugin edits ("plugin edit").
pub(crate) const PLUGIN_EDIT_LABEL: &str = "undo.plugin.edit";

/// The open "plugin edit" undo step: its gesture and the time of its last edit.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EditWindow {
    gesture: GestureId,
    last_ms: u64,
}

/// `ParamEdited`s of one poll not applied yet: first-touch order, last value wins.
#[derive(Default)]
struct PendingEdits {
    order: Vec<(DeviceId, ParamId)>,
    values: std::collections::HashMap<(DeviceId, ParamId), f64>,
}

impl PendingEdits {
    fn push(&mut self, device: DeviceId, param: ParamId, value: f64) {
        if self.values.insert((device, param), value).is_none() {
            self.order.push((device, param));
        }
    }

    fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Handle one poll's plugin notifications (see the module docs).
    pub(crate) fn plugin_notifications(
        &mut self,
        notes: Vec<(DeviceId, PluginNotification)>,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let mut pending = PendingEdits::default();
        for (device, n) in notes {
            match n {
                PluginNotification::ParamEdited { param, value } => {
                    pending.push(device, param, value);
                }
                other => {
                    self.flush_plugin_edits(&mut pending, now, out);
                    self.plugin_notification(device, other, now, out);
                }
            }
        }
        self.flush_plugin_edits(&mut pending, now, out);
        // Close the "plugin edit" step once the plugin went quiet.
        if let Some(w) = self.plugins.edit_window
            && now.saturating_sub(w.last_ms) >= PLUGIN_EDIT_WINDOW_MS
        {
            self.plugins.edit_window = None;
            if let Some(doc) = self.doc.as_mut() {
                doc.history.end_gesture(w.gesture);
            }
        }
    }

    /// Undo label of edits inside a plugin gesture: "set cutoff" for one known param, else
    /// "set diva parameters".
    fn plugin_param_label(
        &self,
        device: DeviceId,
        edits: &[(ParamId, f64)],
    ) -> crate::doc::UndoLabel {
        use crate::doc::UndoLabel;
        use ether_core::protocol::model::text::TextKey;
        let Some(d) = self
            .doc
            .as_ref()
            .and_then(|doc| doc.project.devices.get(&device))
        else {
            return UndoLabel::keyed(TextKey::new("undo.plugin.set_params").arg("count", 1usize));
        };
        if let [(param, _)] = edits
            && let Some(info) = self
                .engine
                .descriptor(d)
                .and_then(|desc| desc.params.into_iter().find(|i| i.id == *param))
            && !info.name.trim().is_empty()
        {
            return UndoLabel::keyed(
                TextKey::new("undo.device.set_param").arg("param", info.name.trim().to_lowercase()),
            );
        }
        match d.name.trim() {
            "" => {
                UndoLabel::keyed(TextKey::new("undo.plugin.set_params").arg("count", edits.len()))
            }
            name => UndoLabel::keyed(
                TextKey::new("undo.plugin.set_device_params")
                    .arg("device", name)
                    .arg("count", edits.len()),
            ),
        }
    }

    /// The gesture of the open "plugin edit" step (a new one if it expired).
    fn plugin_edit_gesture(&mut self, now: u64) -> GestureId {
        let gesture = match self.plugins.edit_window {
            Some(w) if now.saturating_sub(w.last_ms) < PLUGIN_EDIT_WINDOW_MS => w.gesture,
            stale => {
                if let (Some(w), Some(doc)) = (stale, self.doc.as_mut()) {
                    doc.history.end_gesture(w.gesture);
                }
                self.new_gesture()
            }
        };
        self.plugins.edit_window = Some(EditWindow {
            gesture,
            last_ms: now,
        });
        gesture
    }

    /// Apply the pending edits: one transaction per (device, gesture) group.
    fn flush_plugin_edits(
        &mut self,
        pending: &mut PendingEdits,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        if pending.is_empty() {
            return;
        }
        let order = std::mem::take(&mut pending.order);
        let values = std::mem::take(&mut pending.values);
        // (device, plugin gesture) → its edits, in first-touch order.
        let mut groups: Vec<(EditGroup, Vec<(ParamId, f64)>)> = Vec::new();
        {
            let Some(doc) = self.doc.as_ref() else { return };
            for key @ (device, param) in order {
                let value = values[&key];
                let Some(current) = doc.project.devices.get(&device) else {
                    continue;
                };
                // Plugins re-report unchanged values (some on every tick, untouched): such
                // an edit would only grow the undo step, mark the project dirty and save.
                if !value.is_finite()
                    || current.params.get(&param) == Some(&value)
                    || crate::plugins::is_modulated(&doc.project, device, param)
                {
                    continue;
                }
                let group = (device, self.plugin_gestures.get(&key).copied());
                match groups.iter_mut().find(|(k, _)| *k == group) {
                    Some((_, edits)) => edits.push((param, value)),
                    None => groups.push((group, vec![(param, value)])),
                }
            }
        }
        for ((device, plugin_gesture), edits) in groups {
            let (label, gesture) = match plugin_gesture {
                Some(g) => (self.plugin_param_label(device, &edits), g),
                None => (
                    crate::doc::undo(PLUGIN_EDIT_LABEL),
                    self.plugin_edit_gesture(now),
                ),
            };
            // The plugin already has these values: don't echo them back to it.
            self.engine.echo_from = Some(device);
            let _ = self.edit_with(&label, Some(gesture), now, out, |ctx| {
                for (param, value) in edits {
                    ctx.set_device(
                        device,
                        DeviceChange::Param {
                            param,
                            value: Some(value),
                        },
                    )?;
                }
                Ok(())
            });
            self.engine.echo_from = None;
        }
    }
}
