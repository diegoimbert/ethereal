//! Minimal CLAP plugin used by the `ether-clap` / `ether-plugin-scanner` tests (a cdylib
//! example, built with clack's plugin-side crates), so tests never depend on installed plugins.
//!
//! - Stereo in/out audio effect with one CLAP note input port and 64 samples of latency.
//! - Params: `1` Gain (0..2, default 1), `2` Mode (stepped 0..2: Clean/Warm/Hot), `3` Tempo
//!   (read-only, hidden: the last transport tempo it saw).
//! - Output = input * gain, plus 0.25 DC while any note is held, plus the sidechain.
//! - A second, non-main stereo input port `sidechain` (the aux bus of `plugin-sidechain`,
//!   CONTRACTS §12.14): added to the output channel by channel. Hosts feed it silence when
//!   there's no sidechain source, so the output is unchanged then.
//! - MIDI CC 7 sets the gain *from the plugin side* (as a GUI would): it emits gesture begin,
//!   param value and gesture end output events.
//! - State: gain + mode as 16 little-endian bytes.
//! - GUI: CLAP *floating* window only, headless (no real window). While "open" it runs a
//!   10 ms host timer and reports itself closed (`gui.closed`) on the 3rd tick, which
//!   exercises timers + the editor-closed path.
//! - Entry init aborts the process if the bundle path contains `ether-crash`, and hangs
//!   forever if it contains `ether-hang` (to test scanner crash/timeout handling).

use std::cell::Cell;
use std::ffi::CStr;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use clack_extensions::audio_ports::{
    AudioPortFlags, AudioPortInfo, AudioPortInfoWriter, AudioPortType, PluginAudioPorts,
    PluginAudioPortsImpl,
};
use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, HostGui, PluginGui, PluginGuiImpl, Window,
};
use clack_extensions::latency::{PluginLatency, PluginLatencyImpl};
use clack_extensions::note_ports::{
    NoteDialect, NoteDialects, NotePortInfo, NotePortInfoWriter, PluginNotePorts,
    PluginNotePortsImpl,
};
use clack_extensions::params::{
    ParamDisplayWriter, ParamInfo, ParamInfoFlags, ParamInfoWriter, PluginAudioProcessorParams,
    PluginMainThreadParams, PluginParams,
};
use clack_extensions::state::{PluginState, PluginStateImpl};
use clack_extensions::timer::{HostTimer, PluginTimer, PluginTimerImpl, TimerId};
use clack_plugin::entry::prelude::*;
use clack_plugin::events::event_types::{
    ParamGestureBeginEvent, ParamGestureEndEvent, ParamValueEvent,
};
use clack_plugin::events::spaces::CoreEventSpace;
use clack_plugin::prelude::*;
use clack_plugin::stream::{InputStream, OutputStream};

const GAIN: u32 = 1;
const MODE: u32 = 2;
const TEMPO: u32 = 3;
const MODE_LABELS: [&str; 3] = ["Clean", "Warm", "Hot"];

pub struct TestPlugin;

pub struct Shared {
    gain: AtomicU64,
    mode: AtomicU64,
    tempo: AtomicU64,
}

impl Shared {
    fn get(a: &AtomicU64) -> f64 {
        f64::from_bits(a.load(Ordering::Relaxed))
    }
    fn set(a: &AtomicU64, v: f64) {
        a.store(v.to_bits(), Ordering::Relaxed)
    }
    fn handle(&self, event: &UnknownEvent) {
        if let Some(CoreEventSpace::ParamValue(e)) = event.as_core_event() {
            match e.param_id().map(|id| id.get()) {
                Some(GAIN) => Self::set(&self.gain, e.value().clamp(0.0, 2.0)),
                Some(MODE) => Self::set(&self.mode, e.value().round().clamp(0.0, 2.0)),
                _ => {}
            }
        }
    }
}

impl PluginShared<'_> for Shared {}

pub struct MainThread<'a> {
    shared: &'a Shared,
    host: HostMainThreadHandle<'a>,
    timer: Cell<Option<TimerId>>,
    ticks: Cell<u32>,
}

impl<'a> PluginMainThread<'a, Shared> for MainThread<'a> {}

pub struct Processor<'a> {
    shared: &'a Shared,
    held: i32,
}

impl Plugin for TestPlugin {
    type AudioProcessor<'a> = Processor<'a>;
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&Shared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginNotePorts>()
            .register::<PluginParams>()
            .register::<PluginState>()
            .register::<PluginLatency>()
            .register::<PluginGui>()
            .register::<PluginTimer>();
    }
}

impl DefaultPluginFactory for TestPlugin {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::*;
        PluginDescriptor::new("dev.ethereal.test-plugin", "Ethereal Test Plugin")
            .with_vendor("Ethereal")
            .with_version("1.2.3")
            .with_description("Test fixture")
            .with_features([AUDIO_EFFECT, STEREO])
    }

    fn new_shared(_host: HostSharedHandle<'_>) -> Result<Shared, PluginError> {
        Ok(Shared {
            gain: AtomicU64::new(1f64.to_bits()),
            mode: AtomicU64::new(0f64.to_bits()),
            tempo: AtomicU64::new(0f64.to_bits()),
        })
    }

    fn new_main_thread<'a>(
        host: HostMainThreadHandle<'a>,
        shared: &'a Shared,
    ) -> Result<MainThread<'a>, PluginError> {
        Ok(MainThread {
            shared,
            host,
            timer: Cell::new(None),
            ticks: Cell::new(0),
        })
    }
}

impl<'a> PluginAudioProcessor<'a, Shared, MainThread<'a>> for Processor<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &MainThread<'a>,
        shared: &'a Shared,
        _audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        Ok(Self { shared, held: 0 })
    }

    fn process(
        &mut self,
        process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        if let Some(t) = process.transport {
            Shared::set(&self.shared.tempo, t.tempo);
        }
        // Sidechain channels (port 1), as raw pointers: the port pair below borrows `audio`
        // mutably. They stay valid for this call (host buffers).
        let mut sidechain: [Option<(*const f32, usize)>; 2] = [None; 2];
        if let Some(port) = audio.input_port(1)
            && let Some(ch) = port.channels().ok().and_then(|c| c.into_f32())
        {
            for (c, slot) in sidechain.iter_mut().enumerate() {
                *slot = ch.channel(c as u32).map(|s| (s.as_ptr(), s.len()));
            }
        }
        let sc = |c: usize, i: usize| -> f32 {
            match sidechain[c.min(1)] {
                // SAFETY: a host input buffer of `len` samples, valid during `process`.
                Some((p, len)) if i < len => unsafe { *p.add(i) },
                _ => 0.0,
            }
        };
        let mut pair = audio.port_pair(0).ok_or(PluginError::Message("no port"))?;
        let mut channels = pair
            .channels()?
            .into_f32()
            .ok_or(PluginError::Message("expected f32"))?;

        for batch in events.input.batch() {
            for event in batch.events() {
                self.shared.handle(event);
                match event.as_core_event() {
                    Some(CoreEventSpace::NoteOn(_)) => self.held += 1,
                    Some(CoreEventSpace::NoteOff(e)) => {
                        if e.pckn().key.into_specific().is_none() {
                            self.held = 0;
                        } else {
                            self.held = (self.held - 1).max(0);
                        }
                    }
                    Some(CoreEventSpace::Midi(e)) => {
                        let [status, cc, v] = e.data();
                        if status & 0xF0 == 0xB0 && cc == 7 {
                            let value = f64::from(v) / 127.0 * 2.0;
                            Shared::set(&self.shared.gain, value);
                            let time = event.header().time();
                            let id = ClapId::new(GAIN);
                            let _ = events
                                .output
                                .try_push(ParamGestureBeginEvent::new(time, id));
                            let _ = events.output.try_push(ParamValueEvent::new(
                                time,
                                id,
                                Pckn::match_all(),
                                value,
                            ));
                            let _ = events.output.try_push(ParamGestureEndEvent::new(time, id));
                        }
                    }
                    _ => {}
                }
            }
            let gain = Shared::get(&self.shared.gain) as f32;
            let dc = if self.held > 0 { 0.25 } else { 0.0 };
            let bounds = batch.sample_bounds();
            let start = match bounds.0 {
                std::ops::Bound::Included(s) => s,
                std::ops::Bound::Excluded(s) => s + 1,
                std::ops::Bound::Unbounded => 0,
            };
            for (c, pair) in channels.iter_mut().enumerate() {
                match pair {
                    ChannelPair::InputOutput(i, o) => {
                        for (k, (o, i)) in o[bounds]
                            .iter_mut()
                            .zip(&i[bounds])
                            .enumerate()
                        {
                            *o = *i * gain + dc + sc(c, start + k);
                        }
                    }
                    ChannelPair::InPlace(b) => {
                        for (k, s) in b[bounds].iter_mut().enumerate() {
                            *s = *s * gain + dc + sc(c, start + k);
                        }
                    }
                    ChannelPair::OutputOnly(o) => {
                        for (k, s) in o[bounds].iter_mut().enumerate() {
                            *s = dc + sc(c, start + k);
                        }
                    }
                    ChannelPair::InputOnly(_) => {}
                }
            }
        }
        Ok(ProcessStatus::Continue)
    }
}

impl PluginAudioPortsImpl for MainThread<'_> {
    fn count(&self, is_input: bool) -> u32 {
        if is_input { 2 } else { 1 }
    }

    fn get(&self, index: u32, is_input: bool, writer: &mut AudioPortInfoWriter) {
        if is_input && index == 1 {
            writer.set(&AudioPortInfo {
                id: ClapId::new(1),
                name: b"sidechain",
                channel_count: 2,
                flags: AudioPortFlags::empty(),
                port_type: Some(AudioPortType::STEREO),
                in_place_pair: None,
            });
        }
        if index == 0 {
            writer.set(&AudioPortInfo {
                id: ClapId::new(0),
                name: b"main",
                channel_count: 2,
                flags: AudioPortFlags::IS_MAIN,
                port_type: Some(AudioPortType::STEREO),
                in_place_pair: None,
            });
        }
    }
}

impl PluginNotePortsImpl for MainThread<'_> {
    fn count(&self, is_input: bool) -> u32 {
        u32::from(is_input)
    }

    fn get(&self, index: u32, is_input: bool, writer: &mut NotePortInfoWriter) {
        if is_input && index == 0 {
            writer.set(&NotePortInfo {
                id: ClapId::new(0),
                name: b"notes",
                preferred_dialect: Some(NoteDialect::Clap),
                supported_dialects: NoteDialects::CLAP | NoteDialects::MIDI,
            });
        }
    }
}

impl PluginLatencyImpl for MainThread<'_> {
    fn get(&self) -> u32 {
        64
    }
}

impl PluginStateImpl for MainThread<'_> {
    fn save(&self, output: &mut OutputStream) -> Result<(), PluginError> {
        output.write_all(&Shared::get(&self.shared.gain).to_le_bytes())?;
        output.write_all(&Shared::get(&self.shared.mode).to_le_bytes())?;
        Ok(())
    }

    fn load(&self, input: &mut InputStream) -> Result<(), PluginError> {
        let mut buf = [0u8; 16];
        input.read_exact(&mut buf)?;
        let (g, m) = buf.split_at(8);
        Shared::set(
            &self.shared.gain,
            f64::from_le_bytes(g.try_into().unwrap_or_default()),
        );
        Shared::set(
            &self.shared.mode,
            f64::from_le_bytes(m.try_into().unwrap_or_default()),
        );
        Ok(())
    }
}

impl PluginMainThreadParams for MainThread<'_> {
    fn count(&self) -> u32 {
        3
    }

    fn get_info(&self, index: u32, info: &mut ParamInfoWriter) {
        let (id, name, flags, max, default): (u32, &[u8], ParamInfoFlags, f64, f64) = match index {
            0 => (GAIN, b"Gain", ParamInfoFlags::IS_AUTOMATABLE, 2.0, 1.0),
            1 => (
                MODE,
                b"Mode",
                ParamInfoFlags::IS_AUTOMATABLE
                    | ParamInfoFlags::IS_STEPPED
                    | ParamInfoFlags::IS_ENUM,
                2.0,
                0.0,
            ),
            2 => (
                TEMPO,
                b"Tempo",
                ParamInfoFlags::IS_READONLY | ParamInfoFlags::IS_HIDDEN,
                1000.0,
                0.0,
            ),
            _ => return,
        };
        info.set(&ParamInfo {
            id: ClapId::new(id),
            flags,
            cookie: Default::default(),
            name,
            module: b"Main",
            min_value: 0.0,
            max_value: max,
            default_value: default,
        })
    }

    fn get_value(&self, id: ClapId) -> Option<f64> {
        match id.get() {
            GAIN => Some(Shared::get(&self.shared.gain)),
            MODE => Some(Shared::get(&self.shared.mode)),
            TEMPO => Some(Shared::get(&self.shared.tempo)),
            _ => None,
        }
    }

    fn value_to_text(
        &self,
        id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        use std::fmt::Write as _;
        match id.get() {
            MODE => write!(
                writer,
                "{}",
                MODE_LABELS[(value.round().clamp(0.0, 2.0)) as usize]
            ),
            _ => write!(writer, "{value:.2}"),
        }
    }

    fn text_to_value(&self, _id: ClapId, text: &CStr) -> Option<f64> {
        text.to_str().ok()?.trim().parse().ok()
    }

    fn flush(&self, input: &InputEvents, _output: &mut OutputEvents) {
        for event in input {
            self.shared.handle(event);
        }
    }
}

impl PluginAudioProcessorParams for Processor<'_> {
    fn flush(&mut self, input: &InputEvents, _output: &mut OutputEvents) {
        for event in input {
            self.shared.handle(event);
        }
    }
}

impl PluginGuiImpl for MainThread<'_> {
    fn is_api_supported(&self, configuration: GuiConfiguration) -> bool {
        configuration.is_floating
            && Some(configuration.api_type) == GuiApiType::default_for_current_platform()
    }

    fn get_preferred_api(&self) -> Option<GuiConfiguration<'_>> {
        Some(GuiConfiguration {
            api_type: GuiApiType::default_for_current_platform()?,
            is_floating: true,
        })
    }

    fn create(&self, configuration: GuiConfiguration) -> Result<(), PluginError> {
        if !self.is_api_supported(configuration) {
            return Err(PluginError::Message("unsupported gui configuration"));
        }
        let timer_ext: HostTimer = self
            .host
            .get_extension()
            .ok_or(PluginError::Message("host has no timer support"))?;
        let id = timer_ext
            .register_timer(&self.host, 10)
            .map_err(|_| PluginError::Message("register_timer failed"))?;
        self.timer.set(Some(id));
        self.ticks.set(0);
        Ok(())
    }

    fn destroy(&self) {
        if let (Some(id), Some(timer_ext)) =
            (self.timer.take(), self.host.get_extension::<HostTimer>())
        {
            let _ = timer_ext.unregister_timer(&self.host, id);
        }
    }

    fn set_scale(&self, _scale: f64) -> Result<(), PluginError> {
        Ok(())
    }

    fn get_size(&self) -> Option<GuiSize> {
        Some(GuiSize {
            width: 300,
            height: 200,
        })
    }

    fn set_size(&self, _size: GuiSize) -> Result<(), PluginError> {
        Ok(())
    }

    fn set_parent(&self, _window: Window) -> Result<(), PluginError> {
        Err(PluginError::Message("floating only"))
    }

    fn set_transient(&self, _window: Window) -> Result<(), PluginError> {
        Ok(())
    }

    fn show(&self) -> Result<(), PluginError> {
        Ok(())
    }

    fn hide(&self) -> Result<(), PluginError> {
        Ok(())
    }
}

impl PluginTimerImpl for MainThread<'_> {
    fn on_timer(&self, _timer_id: TimerId) {
        let ticks = self.ticks.get() + 1;
        self.ticks.set(ticks);
        if ticks == 3
            && let Some(gui) = self.host.get_extension::<HostGui>()
        {
            gui.closed(&self.host.shared(), false);
        }
    }
}

/// Entry wrapper that can crash or hang on demand (see module docs).
pub struct TestEntry(SinglePluginEntry<TestPlugin>);

impl Entry for TestEntry {
    fn new(bundle_path: Option<&CStr>) -> Result<Self, EntryLoadError> {
        let path = bundle_path
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if path.contains("ether-crash") {
            std::process::abort();
        }
        if path.contains("ether-hang") {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        Ok(Self(SinglePluginEntry::new(bundle_path)?))
    }

    fn declare_factories<'a>(&'a self, builder: &mut EntryFactories<'a>) {
        self.0.declare_factories(builder)
    }
}

clack_export_entry!(TestEntry);
