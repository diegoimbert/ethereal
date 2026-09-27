//! `ether-core`: the real-time engine. Pure DSP.
//!
//! No threads, file system, clock, audio device, logging or allocation on the audio path;
//! compiles natively and to `wasm32-unknown-unknown`. Hosts (`ether-native`, `ether-wasm`)
//! own threads and I/O and drive [`Engine::process`] from their audio callback.
//!
//! # Shape
//!
//! [`engine::create`] returns three halves that live on different threads:
//! - [`Engine`] (audio thread): `process()` renders one block. RT-safe.
//! - [`EngineHandle`] (controller thread): adds nodes/media, publishes snapshots, pushes
//!   parameter changes and transport controls, polls meters/playhead. Never blocks the RT
//!   side; communication is lock-free SPSC rings (`rtrb`).
//! - [`GarbageCollector`] (GC thread, or polled by the controller): receives retired
//!   snapshots/nodes from the audio thread and drops them there.
//!
//! The document never reaches the engine. The controller compiles the model into a
//! plain-data [`RenderGraphDesc`] and publishes it; the handle compiles it (off the audio
//! thread) into an immutable [`RenderSnapshot`] that is swapped in atomically at a block
//! boundary. Stateful processors ([`Node`]s: devices, plugins) are inserted once and
//! referenced by [`NodeKey`], so they keep their state (voices, delay lines) across
//! snapshot swaps.
//!
//! Real-time rules (ARCHITECTURE.md): no alloc/free, no locks shared with non-RT threads, no
//! syscalls/I/O/logging, bounded runtime. Debug tests wrap `process` in `assert_no_alloc`.

pub mod analysis;
pub mod automation;
pub mod automation_rt;
pub mod buffer;
mod bus_tap;
pub mod codec;
pub mod config;
mod delay;
mod drum_rack;
pub mod engine;
pub mod event;
pub mod fades;
pub mod freeze;
pub mod graph;
pub mod media;
pub mod meter;
pub mod metronome;
mod mixer;
pub mod modulation;
pub mod node;
pub mod offline;
pub mod parallel;
pub mod param;
pub mod plugin;
pub mod preview;
pub mod rack_chains;
pub mod recording;
mod sched;
mod sidechain;
pub mod stream_tap;
pub mod tempo;
pub mod transport;
pub mod vca;
mod warp;

pub use analysis::{AnalysisFrame, AnalysisKind};
pub use buffer::AudioBuffers;
pub use bus_tap::InputTapDesc;
pub use config::{EngineConfig, PrepareConfig};
pub use engine::{Engine, EngineError, EngineHandle, EngineParts, GarbageCollector, create};
pub use event::{EventBuffer, EventKind, ProcessEvent};
pub use graph::{CompileError, NodeInfo, RenderGraphDesc, RenderSnapshot};
pub use media::AudioSource;
pub use meter::{EngineOutputs, MeterReading};
pub use node::{Device, Node, NodeKey, ProcessContext, ProcessStatus};
pub use param::{ParamChange, ParamTarget, Smoother};
pub use plugin::{PluginController, PluginNode};
pub use transport::{PlayheadState, TransportControl, TransportInfo};

/// Re-export so hosts/devices don't need a direct dependency for shared types.
pub use ether_protocol as protocol;
pub use ether_stretch::Stretcher;
