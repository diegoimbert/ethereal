//! Top-level wire messages: UI → engine [`ClientMessage`], engine → UI [`ServerMessage`].

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::agent::{AgentCommand, AgentToolSpec};
use crate::analysis::{AnalysisCommand, AnalysisEvent};
use crate::audio_to_midi::{AudioToMidiCommand, AudioToMidiEvent};
use crate::automation::AutomationCommand;
use crate::browser::{BrowserCommand, BrowserEvent, BrowserPage, BrowserRoot};
use crate::capture::{CaptureCommand, CaptureEvent, CaptureResult, CaptureStatus};
use crate::clips::ClipCommand;
use crate::collab::{CollabCommand, CollabEvent};
use crate::devices::FactoryIr;
use crate::devices::{DeviceCommand, DeviceDescriptor};
use crate::drum_rack::{DrumRackCommand, SliceCommand};
use crate::engine::{AudioDeviceList, EngineCommand, EngineEvent, EngineStatus};
use crate::export::{ByteChunk, ExportCommand, ExportEvent, ExportJobId};
use crate::expression::ExpressionCommand;
use crate::external::{ExternalCommand, ExternalEvent, HardwarePorts};
use crate::freeze::{FreezeCommand, FreezeEvent, RenderJobId};
use crate::groove::GrooveCommand;
use crate::keymap::{Keymap, KeymapCommand, KeymapEvent};
use crate::markers::MarkerCommand;
use crate::media::{BrowseRoot, DirectoryListing, MediaCommand, MediaEvent, PeakData};
use crate::media_refs::{MediaRefCommand, MediaRefEvent};
use crate::meters::MeterFrame;
use crate::midi_map::{MidiMapCommand, MidiMapEvent};
use crate::mixer::MixerCommand;
use crate::model::{GestureId, MediaId, MediaRef, MidiMapping, Patch, Project, ProjectId};
use crate::notes::NoteCommand;
use crate::plugins::{PluginCommand, PluginDescriptor, PluginEvent, PluginFolders};
use crate::presets::{PresetCommand, PresetEvent, PresetInfo};
use crate::project::{EditCommand, ProjectCommand, ProjectEvent, ProjectSummary};
use crate::racks::{ModulationCommand, ModulatorDescriptor, RackCommand};
use crate::recording::{InputList, RecordingCommand, RecordingEvent};
use crate::share::{ShareCommand, ShareEvent};
use crate::social::{ChatCommand, PinnedNoteCommand};
use crate::takes::TakeCommand;
use crate::templates::{TemplateCommand, TemplateEvent, TemplateInfo};
use crate::tempo::TempoCommand;
use crate::time_edit::TimeEditCommand;
use crate::tracks::TrackCommand;
use crate::transport::{PlayheadUpdate, TransportCommand, TransportState};
use crate::undo_history::{HistoryCommand, HistoryEvent, HistoryList};
use crate::versions::{RecoveryInfo, VersionCommand, VersionDiff, VersionEvent, VersionInfo};
use crate::warp::WarpCommand;

/// Client-chosen request id, echoed in the [`Reply`]. Unique per connection.
pub type RequestId = u32;

/// UI → engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClientMessage {
    pub id: RequestId,
    /// Commands carrying the same gesture id are merged into one undo step until
    /// `Edit::EndGesture { gesture }` (or a command with another/no gesture) arrives.
    pub gesture: Option<GestureId>,
    pub command: Command,
}

/// Every command, grouped by domain (one Rust file per domain in this crate).
///
/// JSON: `{ "domain": "Mixer", "command": { "type": "SetVolume", "track": "01H…", "volume": -6 } }`
// `Collab(SetPresence)` carries a whole `PresenceState` (it grew with presence v2 and the
// base-62 peer transport). Commands are short-lived, so the size gap doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "domain", content = "command")]
pub enum Command {
    Transport(TransportCommand),
    Project(ProjectCommand),
    Edit(EditCommand),
    Track(TrackCommand),
    Clip(ClipCommand),
    Note(NoteCommand),
    Automation(AutomationCommand),
    Device(DeviceCommand),
    Mixer(MixerCommand),
    Plugin(PluginCommand),
    Recording(RecordingCommand),
    Warp(WarpCommand),
    Media(MediaCommand),
    Engine(EngineCommand),
    // --- roadmap v2 (one domain per feature node, see docs/ROADMAP.md) ---
    Export(ExportCommand),
    Tempo(TempoCommand),
    Marker(MarkerCommand),
    MidiMap(MidiMapCommand),
    Groove(GrooveCommand),
    DrumRack(DrumRackCommand),
    Slice(SliceCommand),
    Collab(CollabCommand),
    // --- v0.2 (contracts-3; one domain per node, see docs/ROADMAP.md "v0.2") ---
    Take(TakeCommand),
    Freeze(FreezeCommand),
    TimeEdit(TimeEditCommand),
    Preset(PresetCommand),
    Browser(BrowserCommand),
    Analysis(AnalysisCommand),
    Rack(RackCommand),
    Modulation(ModulationCommand),
    MediaRef(MediaRefCommand),
    // --- base-62 (`collab-social`, docs/COLLAB.md §12) ---
    /// Session chat (not a document command: never an undo step).
    Chat(ChatCommand),
    /// Notes pinned on the arrangement (document command).
    PinnedNote(PinnedNoteCommand),
    // --- v0.3 (contracts-4; one domain per node, see docs/ROADMAP.md "v0.3") ---
    /// Clip expression lanes, note expressions, track MPE (`midi-expression`, `mpe`).
    Expression(ExpressionCommand),
    /// MIDI capture buffer (`capture-midi`).
    Capture(CaptureCommand),
    /// Audio-to-MIDI conversion jobs (`audio-to-midi`).
    AudioToMidi(AudioToMidiCommand),
    /// External instrument / audio effect routing and latency (`external-instrument`).
    External(ExternalCommand),
    /// Undo history panel (`undo-history`).
    History(HistoryCommand),
    /// Project and track templates (`templates`).
    Template(TemplateCommand),
    /// Project versions and crash recovery (`project-versions`).
    Version(VersionCommand),
    /// User keymap storage (`keymap`).
    Keymap(KeymapCommand),
    // --- base-115 (sharing, docs/SHARING.md) ---
    /// Share / join: P2P host hub, invite links (not a document command).
    Share(ShareCommand),
    // --- agent-api (owner request; docs/MCP.md) ---
    /// LLM agent tools (in-app AI chat, MCP server). Not a document command itself: a tool
    /// call that edits is one undo step.
    Agent(AgentCommand),
}

/// Engine → UI. `Reply` answers exactly one `ClientMessage`; `Event`s are pushed;
/// `Playhead`/`Meters` are the high-rate streams (hosts may deliver them on a separate
/// channel, e.g. a Tauri `Channel` or a SharedArrayBuffer ring).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "body")]
pub enum ServerMessage {
    Reply(Reply),
    Event(Event),
    Playhead(PlayheadFrame),
    Meters(MeterFrame),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PlayheadFrame {
    pub transport: PlayheadUpdate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Reply {
    pub id: RequestId,
    pub result: ReplyResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "status")]
pub enum ReplyResult {
    /// The command was applied. Document changes arrive as `Event::Patch` *before* this
    /// reply, so after `await send(...)` the UI mirror is up to date.
    Ok {
        value: ReplyValue,
    },
    Err {
        error: CommandError,
    },
}

/// Typed payloads of replies. Most commands reply `Unit`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ReplyValue {
    Unit,
    Project {
        project: Box<Project>,
    },
    Projects {
        projects: Vec<ProjectSummary>,
    },
    /// The stored project (after `Save`/`Duplicate`).
    Saved {
        project: ProjectSummary,
    },
    Locations {
        locations: Vec<BrowseRoot>,
    },
    DeviceTypes {
        devices: Vec<DeviceDescriptor>,
    },
    Descriptor {
        descriptor: DeviceDescriptor,
    },
    Plugins {
        plugins: Vec<PluginDescriptor>,
    },
    PluginFolders {
        folders: PluginFolders,
    },
    Media {
        media: MediaRef,
    },
    Peaks {
        peaks: PeakData,
    },
    Directory {
        listing: DirectoryListing,
    },
    Tempo {
        bpm: Option<f64>,
    },
    Inputs {
        inputs: InputList,
    },
    AudioDevices {
        devices: AudioDeviceList,
    },
    Status {
        status: EngineStatus,
    },
    // --- roadmap v2 ---
    ExportStarted {
        job: ExportJobId,
    },
    /// Bulk bytes (export downloads).
    Bytes {
        chunk: ByteChunk,
    },
    MidiMappings {
        mappings: Vec<MidiMapping>,
    },
    // --- v0.2 ---
    /// A freeze/bounce/consolidate render started (`freeze-bounce`).
    RenderStarted {
        job: RenderJobId,
    },
    Presets {
        presets: Vec<PresetInfo>,
    },
    Preset {
        preset: PresetInfo,
    },
    BrowserPage {
        page: BrowserPage,
    },
    BrowserRoots {
        roots: Vec<BrowserRoot>,
    },
    ModulatorKinds {
        kinds: Vec<ModulatorDescriptor>,
    },
    MissingMedia {
        media: Vec<MediaId>,
    },
    // --- v0.3 (contracts-4) ---
    /// `Capture::Capture`.
    Captured {
        capture: CaptureResult,
    },
    /// `Capture::Status`.
    CaptureStatus {
        status: CaptureStatus,
    },
    /// `Device::ListFactoryIrs`.
    FactoryIrs {
        irs: Vec<FactoryIr>,
    },
    /// `External::ListPorts`.
    HardwarePorts {
        ports: HardwarePorts,
    },
    /// `History::{List, JumpTo}`.
    History {
        history: HistoryList,
    },
    Templates {
        templates: Vec<TemplateInfo>,
    },
    Template {
        template: TemplateInfo,
    },
    Versions {
        versions: Vec<VersionInfo>,
    },
    Version {
        version: VersionInfo,
    },
    VersionDiff {
        diff: VersionDiff,
    },
    Recoverable {
        projects: Vec<RecoveryInfo>,
    },
    /// base-131: `Version::SessionStatus`: stored projects whose session marker was left by
    /// another session that didn't close cleanly (a crash, a killed process). Empty = the
    /// previous session closed cleanly.
    SessionStatus {
        unclean: Vec<ProjectId>,
    },
    Keymap {
        keymap: Keymap,
    },
    // --- agent-api ---
    /// `Agent::ListTools`.
    AgentTools {
        tools: Vec<AgentToolSpec>,
    },
    /// `Agent::CallTool`: `content` is JSON or plain text for the model; `is_error` marks a
    /// failed call (unknown tool, invalid input, rejected edit).
    AgentToolResult {
        content: String,
        is_error: bool,
    },
    /// base-114: a packed project bundle (`Project::ExportBundle` without a path), to pull
    /// with `Export::ReadChunk`.
    Bundle {
        download: crate::export::ExportDownload,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ErrorCode {
    /// A referenced entity does not exist.
    NotFound,
    /// Arguments violate a model invariant (e.g. clip on wrong track kind).
    InvalidArgument,
    /// Not available on this host (e.g. plugins on the web).
    Unsupported,
    Io,
    Decode,
    Plugin,
    /// Nothing to undo/redo, deleting the open project, ...
    InvalidState,
    Internal,
}

/// Pushed engine → UI events (low rate).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum Event {
    /// A whole new document (new/open/reconnect). Replaces the UI mirror.
    ProjectLoaded {
        project: Box<Project>,
    },
    /// Project store / current-project lifecycle (list changes, saved, dirty flag).
    Project {
        event: ProjectEvent,
    },
    /// Incremental document change (see `ether_model::patch`).
    Patch {
        patch: Patch,
    },
    Transport {
        state: TransportState,
    },
    Plugin {
        event: PluginEvent,
    },
    Recording {
        event: RecordingEvent,
    },
    Media {
        event: MediaEvent,
    },
    Engine {
        event: EngineEvent,
    },
    // --- roadmap v2 ---
    Export {
        event: ExportEvent,
    },
    MidiMap {
        event: MidiMapEvent,
    },
    Collab {
        event: CollabEvent,
    },
    // --- v0.2 ---
    Freeze {
        event: FreezeEvent,
    },
    Preset {
        event: PresetEvent,
    },
    Browser {
        event: BrowserEvent,
    },
    /// Device analysis frames and modulation readback for watched devices.
    Analysis {
        event: AnalysisEvent,
    },
    MediaRef {
        event: MediaRefEvent,
    },
    // --- v0.3 ---
    Capture {
        event: CaptureEvent,
    },
    AudioToMidi {
        event: AudioToMidiEvent,
    },
    External {
        event: ExternalEvent,
    },
    History {
        event: HistoryEvent,
    },
    Template {
        event: TemplateEvent,
    },
    Version {
        event: VersionEvent,
    },
    Keymap {
        event: KeymapEvent,
    },
    // --- base-115 ---
    /// Sharing state, notices and the web UI peer endpoint (docs/SHARING.md).
    Share {
        event: ShareEvent,
    },
    /// User-facing message (toast).
    Notification {
        level: NotificationLevel,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}
