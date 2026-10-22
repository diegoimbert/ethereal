//! Top-level wire messages: UI → engine [`ClientMessage`], engine → UI [`ServerMessage`].

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::agent::{AgentCommand, AgentToolSpec};
use crate::analysis::{AnalysisCommand, AnalysisEvent};
use crate::audio_to_midi::{AudioToMidiCommand, AudioToMidiEvent};
use crate::automation::AutomationCommand;
use crate::browser::{
    BrowserCommand, BrowserEvent, BrowserPage, BrowserRoot, FileFilter, FileFilterCheck,
};
use crate::capture::{CaptureCommand, CaptureEvent, CaptureResult, CaptureStatus};
use crate::clips::ClipCommand;
use crate::devices::FactoryIr;
use crate::devices::{DeviceCommand, DeviceDescriptor};
use crate::drum_rack::{DrumRackCommand, SliceCommand};
use crate::engine::{AudioDeviceList, EngineCommand, EngineEvent, EngineStatus};
use crate::export::{ByteChunk, ExportCommand, ExportEvent, ExportJobId};
use crate::expression::ExpressionCommand;
use crate::ext::{ExtCommand, ExtValue};
use crate::external::{ExternalCommand, ExternalEvent, HardwarePorts};
use crate::freeze::{FreezeCommand, FreezeEvent, RenderJobId};
use crate::groove::GrooveCommand;
use crate::import::ImportEvent;
use crate::keymap::{Keymap, KeymapCommand, KeymapEvent};
use crate::markers::MarkerCommand;
use crate::media::{BrowseRoot, DirectoryListing, MediaCommand, MediaEvent, PeakData};
use crate::media_refs::{MediaRefCommand, MediaRefEvent};
use crate::meters::MeterFrame;
use crate::midi_map::{MidiMapCommand, MidiMapEvent};
use crate::mixer::MixerCommand;
use crate::model::{GestureId, MediaId, MediaRef, MidiMapping, Patch, Project, ProjectId};
use crate::notes::NoteCommand;
use crate::plugins::{
    PluginCommand, PluginDescriptor, PluginEvent, PluginFolders, SandboxDefault, ScanFailure,
};
use crate::presets::{PresetCommand, PresetEvent, PresetInfo};
use crate::project::{EditCommand, ProjectCommand, ProjectEvent, ProjectSummary};
use crate::racks::{ModulationCommand, ModulatorDescriptor, RackCommand};
use crate::recording::{InputList, RecordingCommand, RecordingEvent};
use crate::resolver::{ReplaceReport, ResolveCommand, ResolveEvent, ResolveProposal};
use crate::samples::{SamplesCommand, SamplesEvent, SamplesView};
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
///
/// A domain this enum doesn't declare (an extension's, docs/EXTENSIONS.md) arrives as
/// [`Command::Ext`].
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
// Serialize/Deserialize: `crate::ext` (the `Ext` variant for the domains not listed here).
#[serde(tag = "domain", content = "command")]
#[serde(remote = "Self")]
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
    // --- roadmap v2 (one domain per feature, CONTRACTS.md §11) ---
    Export(ExportCommand),
    Tempo(TempoCommand),
    Marker(MarkerCommand),
    MidiMap(MidiMapCommand),
    Groove(GrooveCommand),
    DrumRack(DrumRackCommand),
    Slice(SliceCommand),
    // --- v0.2 (one domain per feature, CONTRACTS.md §12) ---
    Take(TakeCommand),
    Freeze(FreezeCommand),
    TimeEdit(TimeEditCommand),
    Preset(PresetCommand),
    Browser(BrowserCommand),
    Analysis(AnalysisCommand),
    Rack(RackCommand),
    Modulation(ModulationCommand),
    MediaRef(MediaRefCommand),
    // --- v0.3 (one domain per feature, CONTRACTS.md §13) ---
    /// Clip expression lanes, note expressions, track MPE (`mpe`).
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
    /// Project versions and crash recovery.
    Version(VersionCommand),
    /// User keymap storage (`keymap`).
    Keymap(KeymapCommand),
    // --- the agent API (docs/MCP.md) ---
    /// LLM agent tools (in-app AI chat, MCP server). Not a document command itself: a tool
    /// call that edits is one undo step.
    Agent(AgentCommand),
    // --- the import resolver (CONTRACTS.md §14.2) ---
    /// Resolve missing plugins and audio files (after an import, or any time). `Apply` is
    /// one undo step.
    Resolve(ResolveCommand),
    // --- i18n (docs/LOCALIZATION.md) ---
    /// App settings the engine takes from the UI (not a document command).
    Settings(SettingsCommand),
    // --- sample pack (docs/SAMPLES.md) ---
    /// The default sample pack: manifest, lazy downloads, the built-in kit (not a document
    /// command).
    Samples(SamplesCommand),
    /// A command of an extension domain (`crate::ext`): any other `domain`. The engine
    /// hands it to its extensions, else replies `Unsupported`.
    #[serde(skip)]
    #[ts(skip)]
    Ext(ExtCommand),
}

/// App settings the engine takes from the UI (never an undo step).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SettingsCommand {
    /// The UI's language (a catalog code: `"en"`, `"fr"`; unknown = English): the engine
    /// names the tracks, devices, rack chains and markers it creates in it ("2 audio",
    /// « réverbération »). Sent at start and on every switch. Replies `Unit`.
    SetLanguage { language: String },
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
// Serialize/Deserialize: `crate::ext` (the `Ext` variant for the types not listed here).
#[serde(tag = "type")]
#[serde(remote = "Self")]
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
    /// `Plugin::ListSandboxDefaults` / `SetSandboxDefault`, in the order added.
    SandboxDefaults {
        plugins: Vec<SandboxDefault>,
    },
    /// `Plugin::ListScanFailures`.
    ScanFailures {
        failed: Vec<ScanFailure>,
    },
    /// `Plugin::ParamTexts`: one entry per value asked (`null` = no text).
    ParamTexts {
        texts: Vec<Option<String>>,
    },
    /// `Plugin::RetryScan`: the plugins found, or why the target failed again.
    ScanRetried {
        plugins: Vec<PluginDescriptor>,
        failure: Option<ScanFailure>,
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
    /// A freeze/bounce/consolidate render started.
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
    /// The current file filter and the defaults (`Browser::{Get,Set,Reset}FileFilter`).
    BrowserFileFilter {
        filter: FileFilter,
        defaults: FileFilter,
    },
    /// `Browser::CheckFileFilter`.
    BrowserFileFilterCheck {
        check: FileFilterCheck,
    },
    ModulatorKinds {
        kinds: Vec<ModulatorDescriptor>,
    },
    MissingMedia {
        media: Vec<MediaId>,
    },
    /// `Samples::{Open, Get}`.
    Samples {
        view: SamplesView,
    },
    // --- v0.3 ---
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
    /// `History::State`.
    HistoryState {
        history: crate::model::HistoryState,
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
    /// `Version::SessionStatus`: stored projects whose session marker was left by
    /// another session that didn't close cleanly (a crash, a killed process). Empty = the
    /// previous session closed cleanly.
    SessionStatus {
        unclean: Vec<ProjectId>,
    },
    Keymap {
        keymap: Keymap,
    },
    // --- agent api ---
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
    /// `Media::ReadMidiFile`.
    MidiFile {
        file: crate::media::MidiFileContents,
    },
    /// `Media::WriteMidiFile`.
    MidiFileWritten {
        result: crate::media::MidiExportResult,
    },
    /// A packed project bundle (`Project::ExportBundle` without a path), to pull
    /// with `Export::ReadChunk`.
    Bundle {
        download: crate::export::ExportDownload,
    },
    /// `Project::ExportTo` finished (report, and the file or a download).
    ProjectExported {
        result: crate::project_export::ProjectExportResult,
    },
    // --- import resolver ---
    /// `Resolve::Propose`.
    ResolveProposal {
        proposal: ResolveProposal,
    },
    /// `Resolve::Replace`.
    PluginReplaced {
        report: ReplaceReport,
    },
    /// A reply value of an extension (`crate::ext`): any other `type`.
    #[serde(skip)]
    #[ts(skip)]
    Ext(ExtValue),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CommandError {
    pub code: ErrorCode,
    /// English, for agents, logs and older clients.
    pub message: String,
    /// The message as a catalog id + arguments, for the UI to show in the user's language
    /// (docs/LOCALIZATION.md); absent for the errors only agents and scripts meet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key: Option<crate::model::text::TextKey>,
}

impl CommandError {
    /// An error with English text only.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            key: None,
        }
    }
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
// Serialize/Deserialize: `crate::ext` (the `Ext` variant for the types not listed here).
#[serde(tag = "type")]
#[serde(remote = "Self")]
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
    /// The default sample pack.
    Samples {
        event: SamplesEvent,
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
    /// `Project::Import` job progress and result.
    Import {
        event: ImportEvent,
    },
    /// `Resolve::SearchMedia` progress and results.
    Resolve {
        event: ResolveEvent,
    },
    /// User-facing message (toast). `message` is English (agents, logs, older clients);
    /// `key` is the same message as a catalog id + arguments, for the UI to show in the
    /// user's language (docs/LOCALIZATION.md).
    Notification {
        level: NotificationLevel,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        key: Option<crate::model::text::TextKey>,
    },
    /// An event of an extension (`crate::ext`): any other `type`.
    #[serde(skip)]
    #[ts(skip)]
    Ext(ExtValue),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}
