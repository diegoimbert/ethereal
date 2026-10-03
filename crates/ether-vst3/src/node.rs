//! Audio-thread half of an in-process VST3 plugin ([`Vst3Node`]).
//!
//! Everything is allocated in [`Vst3Node::new`] (main thread): bus buffers and their
//! channel-pointer arrays, the `IParameterChanges`/`IEventList` objects and the
//! `ProcessContext`. `process` converts engine events (plain param values → normalized,
//! notes → `IEventList`), copies the main input in, calls `IAudioProcessor::process`, and
//! copies the main output back. Param values that reached the processor are sent to the
//! controller (so its `IEditController` and GUI follow automation); edits made in the GUI
//! arrive from the controller and are fed to the processor.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventKind, ProcessEvent};
use ether_core::node::{Device, Node, ProcessContext, ProcessStatus};
use ether_core::plugin::PluginNode;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::expression::mpe::MpeOut;
use ether_core::protocol::model::{NoteExpressionKind, ParamId};
use ether_core::transport::TransportInfo;
use vst3::Steinberg::Vst::ControllerNumbers_::{kAfterTouch, kPitchBend};
use vst3::Steinberg::Vst::Event_::EventTypes_::{
    kNoteExpressionValueEvent, kNoteOffEvent, kNoteOnEvent, kPolyPressureEvent,
};
use vst3::Steinberg::Vst::NoteExpressionTypeIDs_::{kBrightnessTypeID, kTuningTypeID};
use vst3::Steinberg::Vst::ProcessContext_::StatesAndFlags_::{
    kBarPositionValid, kContTimeValid, kCycleActive, kCycleValid, kPlaying, kProjectTimeMusicValid,
    kRecording, kTempoValid, kTimeSigValid,
};
use vst3::Steinberg::Vst::ProcessModes_::kRealtime;
use vst3::Steinberg::Vst::SymbolicSampleSizes_::kSample32;
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, Event, Event__type0, IAudioProcessor,
    IAudioProcessorTrait, IEditController, IEventList, IMidiMapping, IMidiMappingTrait,
    INoteExpressionController, INoteExpressionControllerTrait, IParameterChanges,
    NoteExpressionValueEvent, NoteOffEvent, NoteOnEvent, PolyPressureEvent,
    ProcessContext as Vst3Context, ProcessData,
};
use vst3::Steinberg::{kResultFalse, kResultOk, kResultTrue};
use vst3::{ComPtr, ComWrapper};

use crate::events::{EventList, ParamChanges};
use crate::module::Module;
use crate::params::{StepTable, to_normalized, to_plain};

/// A normalized param value travelling between node and controller.
pub(crate) type ParamMsg = (u32, f64);

/// State shared between the node and its controller.
#[derive(Debug, Default)]
pub(crate) struct NodeShared {
    pub faulted: AtomicBool,
    pub latency: AtomicU32,
}

/// Audio bus channel counts (after `setBusArrangements`) + whether there's an event input.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BusLayout {
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
    pub event_input: bool,
    /// The sidechain bus (CONTRACTS §12.14): the first `kAux` input bus (never the main
    /// bus 0) with channels.
    pub aux_in: Option<usize>,
}

impl BusLayout {
    /// The sidechain bus among the input buses `(channels, is kAux)`.
    pub fn find_aux(inputs: &[(u32, bool)]) -> Option<usize> {
        (1..inputs.len()).find(|&i| inputs[i].1 && inputs[i].0 > 0)
    }

    /// Channels the engine feeds the sidechain bus with: its channel count capped at 2
    /// (the engine's sidechain is stereo). 0 = no sidechain.
    pub fn sidechain_channels(&self) -> u16 {
        self.aux_in
            .and_then(|i| self.inputs.get(i))
            .map_or(0, |ch| (*ch).min(2) as u16)
    }

    /// Main (first) input/output bus channel counts.
    pub fn main_channels(&self) -> (u16, u16) {
        let first = |v: &[u32]| v.first().copied().unwrap_or(0).min(u32::from(u16::MAX)) as u16;
        (first(&self.inputs), first(&self.outputs))
    }
}

/// Buffers of one direction: per bus a `channels * max_frames` block, the channel pointer
/// array the plugin reads, and the `AudioBusBuffers` array itself.
struct Buses {
    data: Vec<Vec<f32>>,
    _ptrs: Vec<Vec<*mut f32>>,
    buses: Vec<AudioBusBuffers>,
}

impl Buses {
    fn new(channels: &[u32], max_frames: usize) -> Self {
        let mut data: Vec<Vec<f32>> = channels
            .iter()
            .map(|c| vec![0.0; *c as usize * max_frames])
            .collect();
        let mut ptrs: Vec<Vec<*mut f32>> = data
            .iter_mut()
            .map(|d| {
                d.chunks_exact_mut(max_frames.max(1))
                    .map(|c| c.as_mut_ptr())
                    .collect()
            })
            .collect();
        let buses = ptrs
            .iter_mut()
            .zip(channels)
            .map(|(p, c)| AudioBusBuffers {
                numChannels: *c as i32,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 {
                    channelBuffers32: p.as_mut_ptr(),
                },
            })
            .collect();
        Self {
            data,
            _ptrs: ptrs,
            buses,
        }
    }
}

/// MIDI controllers a VST3 plugin can map to params (`IMidiMapping`): CC 0..=127,
/// channel aftertouch (`kAfterTouch` = 128) and pitch bend (`kPitchBend` = 129).
const MIDI_CTRLS: usize = 130;

/// v0.3 (`midi-expression`): the plugin's `IMidiMapping` (MIDI controller → param id) per
/// channel, queried once on the main thread when the node is built. VST3 plugins receive
/// CC, pitch bend and channel pressure only as changes of the params they map them to.
#[derive(Clone, Debug, Default)]
pub(crate) struct MidiMap(Option<Box<[[u32; MIDI_CTRLS]; 16]>>);

impl MidiMap {
    const NONE: u32 = u32::MAX;

    /// Main thread: ask the controller for every assignment (empty without `IMidiMapping`).
    pub(crate) fn query(controller: &ComPtr<IEditController>) -> Self {
        let Some(mapping) = controller.cast::<IMidiMapping>() else {
            return Self(None);
        };
        let mut table = Box::new([[Self::NONE; MIDI_CTRLS]; 16]);
        let mut any = false;
        for (ch, row) in table.iter_mut().enumerate() {
            for (ctrl, slot) in row.iter_mut().enumerate() {
                let mut id = 0;
                // SAFETY: valid interface (main thread); `id` is a valid out pointer.
                let r = unsafe {
                    mapping.getMidiControllerAssignment(0, ch as i16, ctrl as i16, &mut id)
                };
                if r == kResultOk || r == kResultTrue {
                    *slot = id;
                    any = true;
                }
            }
        }
        Self(any.then_some(table))
    }

    fn param(&self, channel: u8, ctrl: usize) -> Option<u32> {
        let id = self.0.as_ref()?[usize::from(channel & 0x0F)][ctrl];
        (id != Self::NONE).then_some(id)
    }
}

/// A short MIDI message as VST3 input (besides notes).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Vst3Midi {
    /// A mapped controller (`IMidiMapping` number) at a normalized value.
    Controller {
        channel: u8,
        ctrl: usize,
        value: f64,
    },
    PolyPressure {
        channel: u8,
        key: u8,
        pressure: f32,
    },
}

fn vst3_midi(data: [u8; 3]) -> Option<Vst3Midi> {
    let (status, channel) = (data[0] & 0xF0, data[0] & 0x0F);
    let (d1, d2) = (data[1] & 0x7F, data[2] & 0x7F);
    Some(match status {
        0xA0 => Vst3Midi::PolyPressure {
            channel,
            key: d1,
            pressure: f32::from(d2) / 127.0,
        },
        0xB0 => Vst3Midi::Controller {
            channel,
            ctrl: usize::from(d1),
            value: f64::from(d2) / 127.0,
        },
        0xD0 => Vst3Midi::Controller {
            channel,
            ctrl: kAfterTouch as usize,
            value: f64::from(d1) / 127.0,
        },
        0xE0 => Vst3Midi::Controller {
            channel,
            ctrl: kPitchBend as usize,
            value: f64::from((u16::from(d2) << 7) | u16::from(d1)) / 16383.0,
        },
        _ => return None,
    })
}

pub struct Vst3Node {
    processor: ComPtr<IAudioProcessor>,
    shared: Arc<NodeShared>,
    /// Param values that reached the processor → controller.
    to_main: rtrb::Producer<ParamMsg>,
    /// GUI edits → processor.
    from_main: rtrb::Consumer<ParamMsg>,
    descriptor: DeviceDescriptor,
    layout: BusLayout,
    max_frames: usize,
    sample_rate: f64,
    steps: StepTable,
    /// Current plain values, sorted by id.
    values: Vec<(u32, f64)>,
    /// Per param (parallel to `values`): `(block, offset)` of the last engine event queued,
    /// for the step holds of sample-accurate automation ([`Vst3Node::param_event`]).
    last_event: Vec<(u64, u32)>,
    /// `run` calls so far (`last_event` generation).
    block: u64,
    /// `Device::set_param` calls (and values set while inactive) not yet sent, normalized.
    pending: Vec<ParamMsg>,
    in_changes: ComWrapper<ParamChanges>,
    out_changes: ComWrapper<ParamChanges>,
    in_events: ComWrapper<EventList>,
    out_events: ComWrapper<EventList>,
    context: Box<Vst3Context>,
    ins: Buses,
    outs: Buses,
    /// Sounding notes per channel (bit = key), for `AllNotesOff` / `reset`.
    held: [u128; 16],
    release_all: bool,
    /// v0.3 (`midi-expression`): CC / bend / channel pressure → params.
    midi_map: MidiMap,
    /// v0.3 (`mpe`): the plugin has an `INoteExpressionController` (per-note tuning and
    /// brightness as `NoteExpressionValueEvent`); otherwise MPE MIDI through `mpe_out`
    /// (member channels via `IMidiMapping`).
    note_expressions: bool,
    mpe_out: MpeOut,
    /// Keeps the library loaded while this node (its `IAudioProcessor`) lives, even if the
    /// controller is dropped first. Declared last: dropped after `processor`.
    _module: Arc<Module>,
}

// SAFETY: the raw pointers in `Buses` point into buffers this node owns; the COM objects are
// only used by this node's thread (the processor is the plugin's audio-thread interface).
unsafe impl Send for Vst3Node {}

pub(crate) struct NodeInit {
    pub processor: ComPtr<IAudioProcessor>,
    pub shared: Arc<NodeShared>,
    pub to_main: rtrb::Producer<ParamMsg>,
    pub from_main: rtrb::Consumer<ParamMsg>,
    pub descriptor: DeviceDescriptor,
    pub layout: BusLayout,
    pub config: PrepareConfig,
    pub steps: StepTable,
    pub values: Vec<(u32, f64)>,
    pub pending: Vec<ParamMsg>,
    pub module: Arc<Module>,
    /// v0.3 (`midi-expression`): `MidiMap::query` of the plugin's controller.
    pub midi_map: MidiMap,
    /// v0.3 (`mpe`): [`takes_note_expressions`] of the plugin's controller.
    pub note_expressions: bool,
}

/// v0.3 (`mpe`): whether a controller supports VST3 note expression on its first event
/// bus (`INoteExpressionController` with at least one expression).
pub(crate) fn takes_note_expressions(controller: &ComPtr<IEditController>) -> bool {
    let Some(nec) = controller.cast::<INoteExpressionController>() else {
        return false;
    };
    // SAFETY: valid interface, main thread.
    (0..16).any(|ch| unsafe { nec.getNoteExpressionCount(0, ch) } > 0)
}

/// v0.3 (`mpe`): a `Pitch` in semitones as a normalized VST3 tuning (`kTuningTypeID`:
/// plain = 240 · (normalized − 0.5), i.e. ±120 semitones).
pub(crate) fn tuning_normalized(semitones: f32) -> f64 {
    (0.5 + f64::from(semitones) / 240.0).clamp(0.0, 1.0)
}

impl Vst3Node {
    pub(crate) fn new(init: NodeInit) -> Self {
        let NodeInit {
            processor,
            shared,
            to_main,
            from_main,
            descriptor,
            layout,
            config,
            steps,
            mut values,
            pending: initial,
            module,
            midi_map,
            note_expressions,
        } = init;
        values.sort_by_key(|(id, _)| *id);
        let max_frames = config.max_block_size.max(1);
        let max_events = config.max_events_per_block.max(16);
        let params = values.len().max(1);
        let mut pending = Vec::with_capacity(params.max(16) + initial.len());
        pending.extend(initial);
        // SAFETY: plain C struct; all-zero is a valid "nothing valid" context.
        let context: Box<Vst3Context> = Box::new(unsafe { std::mem::zeroed() });
        Self {
            processor,
            shared,
            to_main,
            from_main,
            descriptor,
            ins: Buses::new(&layout.inputs, max_frames),
            outs: Buses::new(&layout.outputs, max_frames),
            layout,
            max_frames,
            sample_rate: f64::from(config.sample_rate),
            steps,
            last_event: vec![(0, 0); values.len()],
            block: 0,
            values,
            pending,
            // Every param may change in a block; points per param bounded by the event count.
            in_changes: ParamChanges::new(params.max(16), max_events + 1),
            out_changes: ParamChanges::new(params.max(16), 16),
            // Notes + AllNotesOff expansion (up to 128 note-offs).
            in_events: EventList::new(max_events + 128),
            out_events: EventList::new(max_events.max(256)),
            context,
            held: [0; 16],
            release_all: false,
            midi_map,
            note_expressions,
            mpe_out: MpeOut::default(),
            _module: module,
        }
    }

    fn set_value(&mut self, id: u32, plain: f64) {
        if let Ok(i) = self.values.binary_search_by_key(&id, |(k, _)| *k) {
            self.values[i].1 = plain;
        }
    }

    /// An engine param event at sample `offset` (CONTRACTS.md §12.7). VST3 plugins may
    /// interpolate linearly between the points of a param's queue, while the engine's
    /// events are steps (the value changes at that sample, as with CLAP and AU): a point
    /// holding the previous value goes one sample before, unless an event of this block
    /// already sits there.
    fn param_event(&mut self, id: u32, offset: u32, plain: f64) {
        let Some(steps) = self.steps.steps(id) else {
            return;
        };
        let norm = to_normalized(plain, steps);
        if let Ok(i) = self.values.binary_search_by_key(&id, |(k, _)| *k) {
            let (block, at) = self.last_event[i];
            let covered = block == self.block && at + 1 >= offset;
            let prev = to_normalized(self.values[i].1, steps);
            if offset > 0 && !covered && prev != norm {
                self.in_changes.add(id, offset - 1, prev);
            }
            self.last_event[i] = (self.block, offset);
        }
        self.set_value(id, to_plain(norm, steps));
        self.add_param(id, offset, norm);
    }

    fn add_param(&mut self, id: u32, offset: u32, normalized: f64) {
        if self.in_changes.add(id, offset, normalized) {
            let _ = self.to_main.push((id, normalized));
        }
    }

    fn note(&mut self, offset: u32, on: bool, note_id: u32, channel: u8, key: u8, vel: f32) {
        let ch = usize::from(channel.min(15));
        let key = key.min(127);
        let bit = 1u128 << key;
        let (kind, field) = if on {
            self.held[ch] |= bit;
            (
                kNoteOnEvent,
                Event__type0 {
                    noteOn: NoteOnEvent {
                        channel: i16::from(ch as u8),
                        pitch: i16::from(key),
                        tuning: 0.0,
                        velocity: vel,
                        length: 0,
                        noteId: note_id as i32,
                    },
                },
            )
        } else {
            self.held[ch] &= !bit;
            (
                kNoteOffEvent,
                Event__type0 {
                    noteOff: NoteOffEvent {
                        channel: i16::from(ch as u8),
                        pitch: i16::from(key),
                        velocity: vel,
                        noteId: note_id as i32,
                        tuning: 0.0,
                    },
                },
            )
        };
        self.in_events.push(Event {
            busIndex: 0,
            sampleOffset: offset as i32,
            ppqPosition: 0.0,
            flags: 0,
            r#type: kind as u16,
            __field0: field,
        });
    }

    fn poly_pressure(&mut self, offset: u32, note_id: u32, channel: u8, key: u8, pressure: f32) {
        self.in_events.push(Event {
            busIndex: 0,
            sampleOffset: offset as i32,
            ppqPosition: 0.0,
            flags: 0,
            r#type: kPolyPressureEvent as u16,
            __field0: Event__type0 {
                polyPressure: PolyPressureEvent {
                    channel: i16::from(channel.min(15)),
                    pitch: i16::from(key.min(127)),
                    pressure,
                    noteId: note_id as i32,
                },
            },
        });
    }

    /// Note-off for every sounding note (VST3 has no "all notes off" event).
    fn release_held(&mut self, offset: u32) {
        for ch in 0..16u8 {
            while self.held[usize::from(ch)] != 0 {
                let key = self.held[usize::from(ch)].trailing_zeros() as u8;
                // noteId -1 = "no id": matches by channel + pitch.
                self.note(offset, false, u32::MAX, ch, key, 0.0);
            }
        }
    }

    /// v0.3 (`mpe`): a `NoteExpressionValueEvent` for the note `note_id`.
    fn note_expression(&mut self, offset: u32, note_id: u32, type_id: u32, value: f64) {
        self.in_events.push(Event {
            busIndex: 0,
            sampleOffset: offset as i32,
            ppqPosition: 0.0,
            flags: 0,
            r#type: kNoteExpressionValueEvent as u16,
            __field0: Event__type0 {
                noteExpressionValue: NoteExpressionValueEvent {
                    typeId: type_id,
                    noteId: note_id as i32,
                    value,
                },
            },
        });
    }

    /// v0.3 (`mpe`): VST3 note expression when the plugin has it, else MPE MIDI
    /// ([`MpeOut`]: one member channel per note once the track announced its MPE zone).
    fn convert_event(&mut self, e: &ProcessEvent) {
        if self.note_expressions {
            if let EventKind::Midi { data } = e.kind {
                self.mpe_out.observe(data);
            }
            self.convert_one(e.offset, e.kind);
        } else {
            // (Fixed-size state: moved out and back, no allocation.)
            let mut out = std::mem::take(&mut self.mpe_out);
            out.translate(&e.kind, |kind| self.convert_one(e.offset, kind));
            self.mpe_out = out;
        }
    }

    fn convert_one(&mut self, t: u32, kind: EventKind) {
        match kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => self.note(t, true, note_id, channel, key, velocity),
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                velocity,
            } => self.note(t, false, note_id, channel, key, velocity),
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => self.note(t, false, note_id, channel, key, 0.0),
            EventKind::AllNotesOff => self.release_held(t),
            EventKind::Param { param, value } => self.param_event(param.0, t, value),
            EventKind::Midi { data } => {
                let (status, ch) = (data[0] & 0xF0, data[0] & 0x0F);
                let (key, vel) = (data[1] & 0x7F, data[2] & 0x7F);
                let v = f32::from(vel) / 127.0;
                match status {
                    0x90 if vel > 0 => self.note(t, true, u32::MAX, ch, key, v),
                    0x80 | 0x90 => self.note(t, false, u32::MAX, ch, key, v),
                    // v0.3 (`midi-expression`): CC / bend / channel pressure through
                    // `IMidiMapping`, poly pressure as `PolyPressureEvent`.
                    _ => match vst3_midi(data) {
                        Some(Vst3Midi::Controller {
                            channel,
                            ctrl,
                            value,
                        }) => {
                            if let Some(id) = self.midi_map.param(channel, ctrl) {
                                self.add_param(id, t, value);
                            }
                        }
                        Some(Vst3Midi::PolyPressure {
                            channel,
                            key,
                            pressure,
                        }) => self.poly_pressure(t, u32::MAX, channel, key, pressure),
                        None => {}
                    },
                }
            }
            // `Pressure` is VST3's per-note poly pressure (with the note id); `Pitch` and
            // `Timbre` are note expression (tuning, brightness). Without note expression
            // support `MpeOut` already turned them into MIDI.
            EventKind::NoteExpression {
                note_id,
                channel,
                key,
                expression,
                value,
            } => match expression {
                NoteExpressionKind::Pressure => {
                    self.poly_pressure(t, note_id, channel, key, value.clamp(0.0, 1.0))
                }
                NoteExpressionKind::Pitch => {
                    self.note_expression(t, note_id, kTuningTypeID, tuning_normalized(value))
                }
                NoteExpressionKind::Timbre => self.note_expression(
                    t,
                    note_id,
                    kBrightnessTypeID,
                    f64::from(value.clamp(0.0, 1.0)),
                ),
            },
        }
    }

    #[allow(clippy::unnecessary_cast)] // `StatesAndFlags` is `i32` on Windows
    fn fill_context(&mut self, t: &TransportInfo) {
        let sig = t.time_signature;
        let mut state = kTempoValid
            | kTimeSigValid
            | kProjectTimeMusicValid
            | kBarPositionValid
            | kContTimeValid
            | kCycleValid;
        if t.playing {
            state |= kPlaying;
        }
        if t.recording {
            state |= kRecording;
        }
        if t.loop_active {
            state |= kCycleActive;
        }
        let c = &mut *self.context;
        c.state = state as u32;
        c.sampleRate = self.sample_rate;
        c.projectTimeSamples = (t.seconds * self.sample_rate).round() as i64;
        c.continousTimeSamples = t.sample_time as i64;
        c.projectTimeMusic = t.position;
        c.barPositionMusic = t.bar_start;
        c.cycleStartMusic = t.loop_start;
        c.cycleEndMusic = t.loop_end;
        c.tempo = t.bpm;
        c.timeSigNumerator = i32::from(sig.numerator);
        c.timeSigDenominator = i32::from(sig.denominator);
    }

    /// Call `IAudioProcessor::process` with this node's buffers, param changes and events.
    fn call_process(&mut self, frames: usize) -> i32 {
        let (Some(in_changes), Some(out_changes), Some(in_events), Some(out_events)) = (
            self.in_changes.as_com_ref::<IParameterChanges>(),
            self.out_changes.as_com_ref::<IParameterChanges>(),
            self.in_events.as_com_ref::<IEventList>(),
            self.out_events.as_com_ref::<IEventList>(),
        ) else {
            return kResultFalse;
        };
        let mut data = ProcessData {
            processMode: kRealtime as i32,
            symbolicSampleSize: kSample32 as i32,
            numSamples: frames as i32,
            numInputs: self.ins.buses.len() as i32,
            numOutputs: self.outs.buses.len() as i32,
            inputs: self.ins.buses.as_mut_ptr(),
            outputs: self.outs.buses.as_mut_ptr(),
            inputParameterChanges: in_changes.as_ptr(),
            outputParameterChanges: out_changes.as_ptr(),
            inputEvents: if self.layout.event_input {
                in_events.as_ptr()
            } else {
                std::ptr::null_mut()
            },
            outputEvents: out_events.as_ptr(),
            processContext: &mut *self.context,
        };
        // SAFETY: `data` points at buffers/objects owned by this node, sized for `frames`.
        unsafe { self.processor.process(&mut data) }
    }

    /// Deliver pending param values with a zero-sample `process` call (the VST3 way to
    /// "flush" parameters). Main thread, while the processor is active but not in the graph.
    pub(crate) fn flush_params(&mut self) {
        self.in_changes.clear();
        self.in_events.clear();
        self.out_changes.clear();
        self.out_events.clear();
        for i in 0..self.pending.len() {
            let (id, norm) = self.pending[i];
            self.in_changes.add(id, 0, norm);
        }
        self.pending.clear();
        self.call_process(0);
    }
}

impl Node for Vst3Node {
    fn prepare(&mut self, config: &PrepareConfig) {
        if config.max_block_size > self.max_frames {
            tracing::warn!(
                "VST3 node prepared for {} frames but activated for {}",
                config.max_block_size,
                self.max_frames
            );
        }
    }

    fn reset(&mut self) {
        // VST3 has no RT-safe reset; release sounding notes on the next block.
        self.release_all = true;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, &[])
    }

    fn sidechain_inputs(&self) -> u16 {
        self.layout.sidechain_channels()
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, sidechain)
    }

    fn latency(&self) -> u32 {
        self.shared.latency.load(Ordering::Relaxed)
    }

    fn channels(&self) -> (u16, u16) {
        self.layout.main_channels()
    }
}

/// The source of channel `c` of the sidechain bus from the engine's sidechain `sc`
/// (CONTRACTS §12.14): mono buses get the left channel, wider ones the first two (a mono
/// source feeds both), extra channels are silent. `None` = silence.
fn sidechain_source<'a>(sc: &[&'a [f32]], c: usize) -> Option<&'a [f32]> {
    match c {
        0 => sc.first().copied(),
        1 => sc.get(1).or(sc.first()).copied(),
        _ => None,
    }
}

impl Vst3Node {
    /// `process` (no sidechain source: `sidechain` is empty, the aux bus gets silence) and
    /// `process_sidechain`.
    // The SDK enum constants are `u32` or `i32` depending on the OS: keep the casts.
    #[allow(clippy::unnecessary_cast)]
    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        let frames = ctx.frames;
        if self.shared.faulted.load(Ordering::Relaxed) || frames > self.max_frames {
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        if frames == 0 {
            return ProcessStatus::Continue;
        }

        // Inputs: GUI edits and immediate sets (offset 0), then the engine's sorted events.
        self.in_changes.clear();
        self.in_events.clear();
        self.out_changes.clear();
        self.out_events.clear();
        self.block += 1;
        while let Ok((id, norm)) = self.from_main.pop() {
            if let Some(steps) = self.steps.steps(id) {
                self.set_value(id, to_plain(norm, steps));
                // Already known to the controller: don't echo it back.
                self.in_changes.add(id, 0, norm);
            }
        }
        for i in 0..self.pending.len() {
            let (id, norm) = self.pending[i];
            self.add_param(id, 0, norm);
        }
        self.pending.clear();
        if std::mem::take(&mut self.release_all) {
            self.release_held(0);
        }
        for e in ctx.events {
            self.convert_event(e);
        }
        self.fill_context(ctx.transport);

        // Main input → first input bus (mono input feeds every channel); the sidechain → the
        // aux bus; others silent.
        let max = self.max_frames;
        for (b, buf) in self.ins.data.iter_mut().enumerate() {
            let is_aux = self.layout.aux_in == Some(b);
            for (c, dst) in buf.chunks_exact_mut(max).enumerate() {
                let dst = &mut dst[..frames];
                let src = if b == 0 {
                    audio.inputs.get(c).or(audio.inputs.last()).copied()
                } else if is_aux {
                    sidechain_source(sidechain, c)
                } else {
                    None
                };
                match src {
                    Some(src) => dst.copy_from_slice(&src[..frames]),
                    None => dst.fill(0.0),
                }
            }
        }
        for bus in self.ins.buses.iter_mut().chain(self.outs.buses.iter_mut()) {
            bus.silenceFlags = 0;
        }

        let result = self.call_process(frames);
        if result != kResultOk && result != kResultTrue {
            // A failing process() is a fatal plugin error: fault (silence + `Crashed`) rather
            // than silently outputting nothing forever. (Foreign panics cannot be caught.)
            self.shared.faulted.store(true, Ordering::Release);
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }

        // First output bus → node outputs (mono output feeds every channel).
        match self.layout.outputs.first() {
            Some(&ch) if ch > 0 => {
                let src = &self.outs.data[0];
                for (c, dst) in audio.outputs.iter_mut().enumerate() {
                    let c = c.min(ch as usize - 1);
                    dst[..frames].copy_from_slice(&src[c * max..c * max + frames]);
                }
            }
            _ => audio.clear_outputs(),
        }

        // Processor-side param changes (meters, internal edits) → controller.
        let to_main = &mut self.to_main;
        self.out_changes.for_each_last(|id, v| {
            let _ = to_main.push((id, v));
        });
        // Note output.
        let last = (frames - 1) as u32;
        for i in 0..self.out_events.len() {
            let Some(e) = self.out_events.get(i) else {
                continue;
            };
            let offset = (e.sampleOffset.max(0) as u32).min(last);
            let kind = if u32::from(e.r#type) == kNoteOnEvent as u32 {
                // SAFETY: the union member matches the event type.
                let n = unsafe { e.__field0.noteOn };
                EventKind::NoteOn {
                    note_id: n.noteId as u32,
                    channel: n.channel.clamp(0, 15) as u8,
                    key: n.pitch.clamp(0, 127) as u8,
                    velocity: n.velocity,
                }
            } else if u32::from(e.r#type) == kNoteOffEvent as u32 {
                // SAFETY: the union member matches the event type.
                let n = unsafe { e.__field0.noteOff };
                EventKind::NoteOff {
                    note_id: n.noteId as u32,
                    channel: n.channel.clamp(0, 15) as u8,
                    key: n.pitch.clamp(0, 127) as u8,
                    velocity: n.velocity,
                }
            } else {
                continue;
            };
            ctx.out_events.push(ProcessEvent { offset, kind });
        }
        ProcessStatus::Continue
    }
}

impl Device for Vst3Node {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values
            .binary_search_by_key(&id.0, |(k, _)| *k)
            .ok()
            .map(|i| self.values[i].1)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        let Some(steps) = self.steps.steps(id.0) else {
            return;
        };
        let norm = to_normalized(value, steps);
        self.set_value(id.0, to_plain(norm, steps));
        if let Some(p) = self.pending.iter_mut().find(|(k, _)| *k == id.0) {
            p.1 = norm;
        } else if self.pending.len() < self.pending.capacity() {
            self.pending.push((id.0, norm));
        }
    }
}

impl PluginNode for Vst3Node {
    fn is_faulted(&self) -> bool {
        self.shared.faulted.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod midi_tests {
    use super::*;

    #[test]
    fn expression_midi_maps_to_vst3_controllers() {
        assert_eq!(
            vst3_midi([0xB2, 74, 127]),
            Some(Vst3Midi::Controller {
                channel: 2,
                ctrl: 74,
                value: 1.0
            })
        );
        assert_eq!(
            vst3_midi([0xE0, 0, 0x40]),
            Some(Vst3Midi::Controller {
                channel: 0,
                ctrl: kPitchBend as usize,
                value: 8192.0 / 16383.0
            })
        );
        assert_eq!(
            vst3_midi([0xD0, 127, 0]),
            Some(Vst3Midi::Controller {
                channel: 0,
                ctrl: kAfterTouch as usize,
                value: 1.0
            })
        );
        assert_eq!(
            vst3_midi([0xA1, 60, 0]),
            Some(Vst3Midi::PolyPressure {
                channel: 1,
                key: 60,
                pressure: 0.0
            })
        );
        assert_eq!(vst3_midi([0xC0, 1, 0]), None);
        assert_eq!(MidiMap::default().param(0, 1), None);
        let mut t = Box::new([[MidiMap::NONE; MIDI_CTRLS]; 16]);
        t[0][kPitchBend as usize] = 7;
        assert_eq!(MidiMap(Some(t)).param(0, kPitchBend as usize), Some(7));
    }
}

#[cfg(test)]
#[path = "node_mpe_tests.rs"]
mod mpe_tests;
