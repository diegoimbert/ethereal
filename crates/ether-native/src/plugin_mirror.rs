//! Plugin GUI mirrors (`plugin-mirror` node; docs/COLLAB.md §9.6).
//!
//! A **mirror** is a GUI-only instance of a plugin device: instantiated and restored on the
//! main thread like a live one, but **never activated**, never in the engine, never
//! processing. A site listening on a peer hears the host's plugins in the stream, so it
//! swaps its own live instances for mirrors: the user still sees and tweaks the plugin's
//! editor, and every edit reaches the host through the document.
//!
//! - GUI edits come back through [`PluginHost::poll`] (notifications of mirrors are appended
//!   to the live ones) as `ParamEdited` / gestures / `StateDirty` / `EditorClosed`: the
//!   controller turns them into ordinary undoable, replicated `SetParam`s. `LatencyChanged`
//!   and `RestartRequested` are dropped (a mirror has no node, so no latency or restart).
//! - Document param changes are pushed in with [`PluginHost::set_mirror_param`]
//!   (`PluginController::set_param_value`, the inactive-plugin path: CLAP `params.flush`,
//!   VST3 `setParamNormalized` + processor sync, AU parameter tree).
//! - `OpenEditor` / `CloseEditor` fall back to the mirror when the device has no live
//!   instance (`PluginHost::{open_editor, close_editor}` in [`crate::plugins`]).
//! - While a device has a mirror, the bridge gives its engine slot a [`MirrorStandIn`] node
//!   (dry pass-through, or silence for instruments) instead of instantiating the plugin, so
//!   the graph keeps its shape with no plugin processing. Dropping the mirror parks its last
//!   state so the live instance that replaces it starts from what the user did in the GUI.
//!
//! # Threading and re-entrancy
//! Mirror controllers live in a thread-local map on the [`crate::plugins::MainThread`]
//! executor, like the live registry, and follow the same rule: a controller is **checked
//! out** of the map for the duration of a plugin call (editor open can pump a nested run
//! loop), so nested calls see it busy instead of aliasing it. A mirror replaced or destroyed
//! while checked out is dropped at check-in.

use std::cell::RefCell;
use std::collections::HashMap;

use ether_core::plugin::{PluginController, PluginError, PluginNotification};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{DeviceId, ParamId};
use ether_core::{AudioBuffers, Node, PrepareConfig, ProcessContext, ProcessStatus};

use crate::plugins::{Instantiate, PluginHost, PluginSource};

struct Slot {
    /// Distinguishes a mirror from its replacement (a checked-out controller only goes back
    /// into its own slot).
    token: u64,
    /// `None` while checked out for a plugin call.
    controller: Option<Box<dyn PluginController>>,
}

#[derive(Default)]
struct Table {
    slots: HashMap<DeviceId, Slot>,
    next_token: u64,
}

thread_local! {
    /// Mirror controllers (main-thread executor only; never borrowed across a plugin call).
    static MIRRORS: RefCell<Table> = RefCell::new(Table::default());
}

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    MIRRORS.with(|t| f(&mut t.borrow_mut()))
}

fn busy() -> PluginError {
    PluginError::Ipc("plugin mirror is busy (another main-thread call is running)".into())
}

/// Take `device`'s mirror out of the table: `NotFound` without one, busy if checked out.
fn checkout(device: DeviceId) -> Result<(u64, Box<dyn PluginController>), PluginError> {
    with_table(|t| {
        let slot = t
            .slots
            .get_mut(&device)
            .ok_or_else(|| PluginError::NotFound(device.to_string()))?;
        let c = slot.controller.take().ok_or_else(busy)?;
        Ok((slot.token, c))
    })
}

/// Put a checked-out mirror back, or drop it if it was replaced or destroyed meanwhile.
fn checkin(device: DeviceId, token: u64, controller: Box<dyn PluginController>) {
    let stale = with_table(|t| match t.slots.get_mut(&device) {
        Some(slot) if slot.token == token && slot.controller.is_none() => {
            slot.controller = Some(controller);
            None
        }
        _ => Some(controller),
    });
    drop_mirror(stale);
}

/// Drop a mirror controller outside any table borrow (its editor closed first).
fn drop_mirror(controller: Option<Box<dyn PluginController>>) {
    if let Some(mut c) = controller {
        c.close_editor();
        drop(c);
    }
}

fn with_mirror<R>(
    device: DeviceId,
    f: impl FnOnce(&mut dyn PluginController) -> R,
) -> Result<R, PluginError> {
    let (token, mut c) = checkout(device)?;
    let r = f(&mut *c);
    checkin(device, token, c);
    Ok(r)
}

/// Main thread: open `device`'s mirror editor (`NotFound` without a mirror).
pub(crate) fn open_editor_main(device: DeviceId) -> Result<(), PluginError> {
    with_mirror(device, |c| {
        if !c.has_editor() {
            return Err(PluginError::NoEditor);
        }
        c.open_editor()
    })?
}

/// Main thread: close `device`'s mirror editor, if any.
pub(crate) fn close_editor_main(device: DeviceId) {
    let _ = with_mirror(device, |c| c.close_editor());
}

/// Main thread: poll every mirror (GUI timers, CLAP `on_main_thread`, param flushes,
/// VST3 component-handler edits) and append their notifications. Checked-out mirrors are
/// skipped this time; notifications a node-less instance cannot act on are dropped.
pub(crate) fn poll_main(out: &mut Vec<(DeviceId, PluginNotification)>) {
    let devices: Vec<DeviceId> = with_table(|t| {
        t.slots
            .iter()
            .filter(|(_, s)| s.controller.is_some())
            .map(|(d, _)| *d)
            .collect()
    });
    let mut buf = Vec::new();
    for device in devices {
        if with_mirror(device, |c| c.poll(&mut buf)).is_ok() {
            out.extend(
                buf.drain(..)
                    .filter(|n| {
                        !matches!(
                            n,
                            PluginNotification::LatencyChanged { .. }
                                | PluginNotification::RestartRequested
                        )
                    })
                    .map(|n| (device, n)),
            );
        }
        buf.clear();
    }
}

impl PluginHost {
    /// Instantiate a GUI-only mirror of `device` and restore `state` (main thread; never
    /// activated). An existing mirror of the device is replaced. Returns the descriptor.
    pub fn create_mirror(
        &self,
        instantiate: Instantiate,
        device: DeviceId,
        source: PluginSource,
        state: Option<Vec<u8>>,
    ) -> Result<DeviceDescriptor, PluginError> {
        self.call(move || {
            // No table borrow from here on: the plugin may pump a nested run loop.
            let mut controller = instantiate(source.format, &source.path, &source.plugin_id)?;
            if let Some(state) = state.as_deref().filter(|s| !s.is_empty()) {
                controller.load_state(state)?;
            }
            let descriptor = controller.descriptor();
            let old = with_table(|t| {
                let token = t.next_token;
                t.next_token += 1;
                t.slots.insert(
                    device,
                    Slot {
                        token,
                        controller: Some(controller),
                    },
                )
            });
            drop_mirror(old.and_then(|s| s.controller));
            Ok(descriptor)
        })?
    }

    /// Drop `device`'s mirror (its editor closed first). Returns its last state, for the
    /// live instance that replaces it. Unknown devices: `Ok(None)`.
    pub fn destroy_mirror(&self, device: DeviceId) -> Result<Option<Vec<u8>>, PluginError> {
        self.call(move || {
            let state = with_mirror(device, |c| c.save_state().ok()).ok().flatten();
            let slot = with_table(|t| t.slots.remove(&device));
            // Checked out by a nested call: dropped at its check-in instead.
            drop_mirror(slot.and_then(|s| s.controller));
            state
        })
    }

    /// Show a document param value in `device`'s mirror (inactive-plugin param path).
    pub fn set_mirror_param(
        &self,
        device: DeviceId,
        param: ParamId,
        value: f64,
    ) -> Result<(), PluginError> {
        self.call(move || with_mirror(device, |c| c.set_param_value(param, value))?)?
    }

    /// Current state blob of `device`'s mirror (`None` without one).
    pub fn mirror_state(&self, device: DeviceId) -> Result<Option<Vec<u8>>, PluginError> {
        self.call(move || match with_mirror(device, |c| c.save_state()) {
            Ok(r) => r.map(Some),
            Err(PluginError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        })?
    }

    /// Current plain value of a mirror param (tests/diagnostics).
    pub fn mirror_param(&self, device: DeviceId, param: ParamId) -> Option<f64> {
        self.call(move || with_mirror(device, |c| c.param_value(param)).ok().flatten())
            .ok()
            .flatten()
    }

    /// Mirrors alive on the main thread (tests/diagnostics).
    pub fn mirror_count(&self) -> usize {
        self.call(|| with_table(|t| t.slots.len())).unwrap_or(0)
    }
}

/// Bridge-side bookkeeping of the mirrors ([`crate::NativeBridge`], controller thread).
#[derive(Debug, Default)]
pub struct Mirrors {
    /// Descriptor of each mirrored device (its engine slot is a [`MirrorStandIn`]).
    descriptors: HashMap<DeviceId, DeviceDescriptor>,
    /// Last value each mirror param is known to show (pushed in, or edited in its GUI):
    /// the document echo of a GUI edit is not pushed back into the plugin mid-gesture.
    known: HashMap<(DeviceId, ParamId), f64>,
    /// Last state of a dropped mirror, until the live instance replacing it is created.
    parked: HashMap<DeviceId, Vec<u8>>,
}

impl Mirrors {
    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    pub fn contains(&self, device: DeviceId) -> bool {
        self.descriptors.contains_key(&device)
    }

    pub fn descriptor(&self, device: DeviceId) -> Option<&DeviceDescriptor> {
        self.descriptors.get(&device)
    }

    pub(crate) fn insert(&mut self, device: DeviceId, descriptor: DeviceDescriptor) {
        self.descriptors.insert(device, descriptor);
        self.known.retain(|(d, _), _| *d != device);
        self.parked.remove(&device);
    }

    pub(crate) fn remove(&mut self, device: DeviceId) {
        self.descriptors.remove(&device);
        self.known.retain(|(d, _), _| *d != device);
    }

    pub(crate) fn park(&mut self, device: DeviceId, state: Vec<u8>) {
        self.parked.insert(device, state);
    }

    pub(crate) fn parked(&self, device: DeviceId) -> Option<&Vec<u8>> {
        self.parked.get(&device)
    }

    /// The device's engine slot is gone or holds a live instance again.
    pub(crate) fn unpark(&mut self, device: DeviceId) {
        self.parked.remove(&device);
    }

    /// Whether pushing `value` would change what the mirror shows; records it.
    pub(crate) fn push(&mut self, device: DeviceId, param: ParamId, value: f64) -> bool {
        match self.known.insert((device, param), value) {
            Some(old) => old != value,
            None => true,
        }
    }

    /// Forget a push that did not reach the plugin (so it is retried).
    pub(crate) fn forget(&mut self, device: DeviceId, param: ParamId) {
        self.known.remove(&(device, param));
    }

    /// Record the values mirrors reported from their GUIs.
    pub(crate) fn observe(&mut self, notes: &[(DeviceId, PluginNotification)]) {
        for (device, n) in notes {
            if let PluginNotification::ParamEdited { param, value } = n
                && self.descriptors.contains_key(device)
            {
                self.known.insert((*device, *param), *value);
            }
        }
    }
}

/// The engine node of a mirrored device: the plugin does not process, its slot passes the
/// dry signal through (silence for channels without an input, e.g. instruments). Keeps the
/// device's channel layout (and sidechain input count) so the graph compiles unchanged.
pub struct MirrorStandIn {
    channels: (u16, u16),
    sidechain: u16,
}

impl MirrorStandIn {
    pub fn new(descriptor: &DeviceDescriptor) -> Self {
        Self {
            channels: (descriptor.audio_inputs, descriptor.audio_outputs),
            sidechain: descriptor.sidechain_inputs,
        }
    }
}

impl Node for MirrorStandIn {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.pass(audio)
    }
    fn channels(&self) -> (u16, u16) {
        self.channels
    }
    fn sidechain_inputs(&self) -> u16 {
        self.sidechain
    }
}

impl MirrorStandIn {
    /// RT-safe: outputs = inputs (channels the device has an input for), else silence.
    fn pass(&self, audio: &mut AudioBuffers<'_, '_>) -> ProcessStatus {
        let mut signal = false;
        for (c, out) in audio.outputs.iter_mut().enumerate() {
            match audio.inputs.get(c) {
                Some(input) if c < usize::from(self.channels.0) => {
                    let n = out.len().min(input.len());
                    out[..n].copy_from_slice(&input[..n]);
                    out[n..].fill(0.0);
                    signal = true;
                }
                _ => out.fill(0.0),
            }
        }
        if signal {
            ProcessStatus::Continue
        } else {
            ProcessStatus::Silent
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::DedicatedThread;
    use ether_core::protocol::devices::{
        DeviceCategory, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
    };
    use ether_core::protocol::model::{PluginFormat, Ulid};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    fn device(n: u128) -> DeviceId {
        DeviceId(Ulid(n))
    }

    fn src() -> PluginSource {
        PluginSource {
            format: PluginFormat::Clap,
            path: PathBuf::from("/fake.clap"),
            plugin_id: "fake".into(),
        }
    }

    /// Shared view of a fake mirror (the controller itself lives on the main thread).
    #[derive(Default)]
    struct Probe {
        state: Mutex<Vec<u8>>,
        params: Mutex<Vec<(ParamId, f64)>>,
        /// GUI edits to report at the next poll.
        edits: Mutex<Vec<PluginNotification>>,
        editor_open: AtomicBool,
        activated: AtomicBool,
        dropped: AtomicBool,
    }

    struct Fake(Arc<Probe>);

    impl Drop for Fake {
        fn drop(&mut self) {
            self.0.dropped.store(true, Ordering::SeqCst);
        }
    }

    fn descriptor() -> DeviceDescriptor {
        DeviceDescriptor {
            layout: None,
            device_type: DeviceTypeRef::Plugin {
                plugin_id: "fake".into(),
            },
            name: "Fake".into(),
            category: DeviceCategory::AudioEffect,
            params: vec![ParamInfo {
                id: ParamId(1),
                name: "Gain".into(),
                group: None,
                unit: ParamUnit::None,
                min: 0.0,
                max: 2.0,
                default: 1.0,
                scale: ParamScale::Linear,
                labels: None,
                automatable: true,
                hidden: false,
                step: None,
            }],
            audio_inputs: 2,
            audio_outputs: 2,
            midi_input: false,
            sidechain_inputs: 0,
        }
    }

    impl PluginController for Fake {
        fn descriptor(&self) -> DeviceDescriptor {
            descriptor()
        }
        fn params(&mut self) -> Vec<ParamInfo> {
            descriptor().params
        }
        fn activate(
            &mut self,
            _: &PrepareConfig,
        ) -> Result<Box<dyn ether_core::plugin::PluginNode>, PluginError> {
            self.0.activated.store(true, Ordering::SeqCst);
            Err(PluginError::Activation("a mirror is never activated".into()))
        }
        fn deactivate(&mut self, _: Box<dyn ether_core::plugin::PluginNode>) {}
        fn save_state(&mut self) -> Result<Vec<u8>, PluginError> {
            Ok(self.0.state.lock().unwrap().clone())
        }
        fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
            *self.0.state.lock().unwrap() = state.to_vec();
            Ok(())
        }
        fn has_editor(&self) -> bool {
            true
        }
        fn open_editor(&mut self) -> Result<(), PluginError> {
            self.0.editor_open.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn close_editor(&mut self) {
            self.0.editor_open.store(false, Ordering::SeqCst);
        }
        fn poll(&mut self, out: &mut Vec<PluginNotification>) {
            out.append(&mut self.0.edits.lock().unwrap());
        }
        fn param_value(&mut self, param: ParamId) -> Option<f64> {
            self.0
                .params
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(p, _)| *p == param)
                .map(|(_, v)| *v)
        }
        fn set_param_value(&mut self, param: ParamId, value: f64) -> Result<(), PluginError> {
            self.0.params.lock().unwrap().push((param, value));
            Ok(())
        }
    }

    fn fake(probe: &Arc<Probe>) -> Instantiate {
        let probe = probe.clone();
        Arc::new(move |_, _, id| {
            if id == "missing" {
                return Err(PluginError::NotFound(id.into()));
            }
            Ok(Box::new(Fake(probe.clone())) as Box<dyn PluginController>)
        })
    }

    #[test]
    fn mirror_lifecycle_never_activates() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let probe = Arc::new(Probe::default());
        let d = device(1);
        let desc = host
            .create_mirror(fake(&probe), d, src(), Some(b"preset".to_vec()))
            .unwrap();
        assert_eq!(desc.name, "Fake");
        assert_eq!(host.mirror_count(), 1);
        assert_eq!(host.live_count(), 0, "not in the live registry");
        assert!(!probe.activated.load(Ordering::SeqCst));
        assert_eq!(host.mirror_state(d).unwrap().as_deref(), Some(&b"preset"[..]));

        // Document values pushed in (inactive path).
        host.set_mirror_param(d, ParamId(1), 0.25).unwrap();
        assert_eq!(host.mirror_param(d, ParamId(1)), Some(0.25));

        // Editor routing: no live instance → the mirror's editor.
        host.open_editor(d).unwrap();
        assert!(probe.editor_open.load(Ordering::SeqCst));
        host.close_editor(d).unwrap();
        assert!(!probe.editor_open.load(Ordering::SeqCst));

        // GUI edits come back through the ordinary poll; node-only notifications don't.
        probe.edits.lock().unwrap().extend([
            PluginNotification::GestureBegin { param: ParamId(1) },
            PluginNotification::ParamEdited {
                param: ParamId(1),
                value: 1.5,
            },
            PluginNotification::LatencyChanged { samples: 64 },
            PluginNotification::RestartRequested,
            PluginNotification::GestureEnd { param: ParamId(1) },
        ]);
        let mut notes = Vec::new();
        host.poll(&mut notes);
        assert_eq!(
            notes,
            vec![
                (d, PluginNotification::GestureBegin { param: ParamId(1) }),
                (
                    d,
                    PluginNotification::ParamEdited {
                        param: ParamId(1),
                        value: 1.5
                    }
                ),
                (d, PluginNotification::GestureEnd { param: ParamId(1) }),
            ]
        );

        // Destroy: editor closed, controller dropped, last state returned.
        host.open_editor(d).unwrap();
        *probe.state.lock().unwrap() = b"edited".to_vec();
        assert_eq!(
            host.destroy_mirror(d).unwrap().as_deref(),
            Some(&b"edited"[..])
        );
        assert!(!probe.editor_open.load(Ordering::SeqCst));
        assert!(probe.dropped.load(Ordering::SeqCst));
        assert_eq!(host.mirror_count(), 0);
        assert!(matches!(host.open_editor(d), Err(PluginError::NotFound(_))));
        assert_eq!(host.destroy_mirror(d).unwrap(), None, "unknown: no-op");
        assert_eq!(host.mirror_state(d).unwrap(), None);
        assert!(host.set_mirror_param(d, ParamId(1), 1.0).is_err());
        assert!(!probe.activated.load(Ordering::SeqCst));

        let err = host.create_mirror(
            fake(&probe),
            d,
            PluginSource {
                plugin_id: "missing".into(),
                ..src()
            },
            None,
        );
        assert!(matches!(err, Err(PluginError::NotFound(_))));
        assert_eq!(host.mirror_count(), 0);
    }

    #[test]
    fn replacing_a_mirror_drops_the_old_one() {
        let host = PluginHost::new(Arc::new(DedicatedThread::new()));
        let (a, b) = (Arc::new(Probe::default()), Arc::new(Probe::default()));
        let d = device(2);
        host.create_mirror(fake(&a), d, src(), None).unwrap();
        host.open_editor(d).unwrap();
        host.create_mirror(fake(&b), d, src(), None).unwrap();
        assert!(a.dropped.load(Ordering::SeqCst));
        assert!(!a.editor_open.load(Ordering::SeqCst), "closed before drop");
        assert!(!b.dropped.load(Ordering::SeqCst));
        assert_eq!(host.mirror_count(), 1);
    }

    #[test]
    fn bookkeeping_dedupes_pushes_and_gui_echoes() {
        let mut m = Mirrors::default();
        let d = device(3);
        m.insert(d, descriptor());
        assert!(m.push(d, ParamId(1), 0.5));
        assert!(!m.push(d, ParamId(1), 0.5), "same value: not pushed again");
        // The GUI moved the knob: the document echo of that edit is not pushed back.
        m.observe(&[(
            d,
            PluginNotification::ParamEdited {
                param: ParamId(1),
                value: 0.75,
            },
        )]);
        assert!(!m.push(d, ParamId(1), 0.75));
        assert!(m.push(d, ParamId(1), 0.8));
        m.forget(d, ParamId(1));
        assert!(m.push(d, ParamId(1), 0.8), "retried after a failed push");
        // Other devices' notifications are ignored.
        m.observe(&[(
            device(4),
            PluginNotification::ParamEdited {
                param: ParamId(1),
                value: 0.1,
            },
        )]);
        assert!(!m.known.contains_key(&(device(4), ParamId(1))));
        m.park(d, b"s".to_vec());
        m.remove(d);
        assert!(m.is_empty());
        assert_eq!(m.parked(d).map(Vec::as_slice), Some(&b"s"[..]));
        m.unpark(d);
        assert!(m.parked(d).is_none());
    }

    #[test]
    fn stand_in_passes_the_dry_signal() {
        let run = |node: &mut MirrorStandIn, inputs: &[&[f32]], outs: usize| {
            let mut out = vec![vec![9.0f32; 4]; outs];
            let mut outputs: Vec<&mut [f32]> = out.iter_mut().map(|v| v.as_mut_slice()).collect();
            let mut audio = AudioBuffers {
                inputs,
                outputs: &mut outputs,
            };
            let status = node.pass(&mut audio);
            (out, status)
        };
        let mut fx = MirrorStandIn::new(&descriptor());
        assert_eq!(fx.channels(), (2, 2));
        let (out, status) = run(&mut fx, &[&[1.0; 4], &[0.5; 4]], 2);
        assert_eq!(out, vec![vec![1.0; 4], vec![0.5; 4]]);
        assert_eq!(status, ProcessStatus::Continue);
        let mut synth = MirrorStandIn::new(&DeviceDescriptor {
            audio_inputs: 0,
            sidechain_inputs: 2,
            ..descriptor()
        });
        assert_eq!(synth.sidechain_inputs(), 2);
        let (out, status) = run(&mut synth, &[], 2);
        assert_eq!(out, vec![vec![0.0; 4]; 2]);
        assert_eq!(status, ProcessStatus::Silent);
    }
}
