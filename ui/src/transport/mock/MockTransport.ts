/**
 * MockTransport: an in-memory engine implementing `EngineTransport` with no Rust, so UI
 * nodes can build real features (and tests) against it. It is also the executable
 * reference of the transport contract; real hosts must behave the same way.
 *
 * ## Semantics (shared with the real engine)
 * - **Document**: a normalized `Project` (flat tables keyed by ULID, sibling order via
 *   fractional `OrderKey`s; the project itself is identified by a UUIDv7 `ProjectId`).
 *   `connect()` returns a copy; afterwards the UI only learns about changes through events.
 * - **Patches before replies**: a document command applies atomically, then emits ONE
 *   `Event::Patch` (whole-entity `Upsert`/`Remove`/`Settings` changes, `revision` + 1,
 *   current `HistoryState`), and only then settles the `send()` promise. After
 *   `await send(...)` the store mirror already reflects the command.
 * - **Errors**: failing commands change nothing and reject with `CommandFailedError`.
 * - **Client-side ids**: commands that create entities carry their ids (`newId()`, or
 *   `newProjectId()` for projects); the engine only generates ids for entities it creates
 *   on its own (children of duplicated tracks/clips, the right part of an overlap
 *   split).
 * - **Undo**: every document command is one undo step. Commands sent with the same
 *   `gesture` merge into one step until `Edit::EndGesture` (or a command with another/no
 *   gesture). `Edit::Batch` applies several document commands as one all-or-nothing step
 *   (no transport commands, queries or nested batches inside). Undo/Redo emit patches.
 * - **Engine-side files only**: the UI may run on another machine, so it never sends file
 *   paths or file contents. Projects live in an engine-side store addressed by
 *   `ProjectId` (here: an in-memory map `ProjectId → saved .ether JSON + modified_ms`,
 *   seeded with "Demo", "Beat sketch" and "Ambient idea"; "Demo" is opened on connect).
 *   `Project::List/Create/Open/Save/SaveAs/Duplicate/Rename/Delete` operate on it and emit
 *   `Event::Project` (`ListChanged`, `Saved`, `DirtyChanged`). Renaming the *current*
 *   project is an undoable document edit (settings name) that also updates the list.
 *   Opening/creating another project autosaves a dirty current one first. The dirty flag
 *   is set by any document patch and cleared by saves/loads.
 * - **Media**: browsed through engine-visible locations (`Media::ListLocations`: a fake
 *   "Library" of wav files and the current project's `media/` folder) with relative
 *   paths. `Media::Import` of a library file references it in place
 *   (`MediaLocation::External`; uploads are "copied" into the project:
 *   `MediaRef.file` = `media/<id>-<name>`), delivered as an undoable `Media` upsert patch.
 *   `mediaRefs.setOffline(path)` simulates a moved sample (missing on the next open).
 * - **Record-arm** is runtime state, not document: `Recording::Arm` is not undoable and is
 *   reported as `Event::Recording { ArmChanged { armed } }` on change and on connect.
 * - **Routing**: `TrackOutput::Default` = the parent group's bus for tracks inside a group,
 *   the master otherwise (the meters follow this). Groups nest via `Track.parent`;
 *   deleting a group deletes its children.
 * - **Non-document state**: transport play state → `Event::Transport` (emitted on connect
 *   and whenever a field changes);
 *   playhead (~60 Hz while playing) and meters (~30 Hz) on their own streams.
 * - **Beats** are `f64`; all grid math uses the shared helpers of `@/state/beats`.
 * - Everything crossing the "wire" is JSON-cloned, like a real host would serialize it.
 * - On connect the mock emits, in order: `Transport`, `Recording::ArmChanged`,
 *   `Project::ListChanged`, `Project::DirtyChanged`.
 * - **New audio clips are unwarped** (`warp.enabled = false`, Repitch), like the real
 *   controller.
 *
 * ## Roadmap v2 simulations (one file per feature in `./roadmap/`)
 * - Undoable document commands: `Tempo::*` (tempo/signature CRUD; the points at beat 0
 *   can't be removed or moved; metronome settings), `Marker::*`, `Clip::{SetFadeCurves,
 *   SetReversed, Crossfade}` (the crossfade just extends the first clip), `Device::
 *   SetSidechain`, `Groove::{Humanize (seeded, deterministic), SetSwing}`, quantize swing,
 *   `DrumRack::*` (pads + pad chains), `Slice::*` (auto slicing: `Equal`/`Grid`;
 *   `Transients` = 8 equal slices; `ToDrumRack`, `SampleToPads`), `MidiMap::{Map, Edit,
 *   Unmap}`. Deleting devices/sends/tracks cascades to MIDI mappings, drum pads and
 *   sidechain sources like the Rust controller.
 * - `MidiMap::Learn` arms learning (`Event::MidiMap LearnChanged`); feed input with
 *   `simulateMidiInput(port, [status, d1, d2])` (completes a learn, or drives the mapped
 *   target). `MidiMap::List` replies `MidiMappings`.
 * - `Export::Render` replies `ExportStarted`, then advances one phase per playhead step
 *   (16 ms, or `tick()` with manual timers): `Progress 0.5`, then `Progress 1` + `Done`
 *   with `Download` results (silent 0.1 s WAV bytes, also for FLAC) readable with
 *   `Export::ReadChunk`. `Cancel`/`Release` as specified; FLAC + Float32 is rejected.
 *
 * ## Mock limitations
 * - Tempo map is step-only (linear tempo ramps are treated as steps).
 * - No audio. Meters are synthesized from what "would" play (clips under the playhead,
 *   volume/pan/mute, sends, group/default routing); CPU load is
 *   fake. Library files only have metadata; peaks are synthesized deterministically.
 * - Uploads (`Media::{BeginUpload, UploadChunk, CancelUpload}`, `MediaSource::Upload`) are
 *   staged in memory (`roadmap/remote.ts`); `MediaSource::Path` replies `Unsupported`.
 * - Replies `Err { code: "Unsupported" }`: plugins (insert/editor/sandbox/reload),
 *   `Engine::SetAudioConfig`.
 * - `Recording::SetRecording` simulates recording with its live view (`roadmap/liveRecord.ts`).
 * - Harmless answers: `Plugin::List` → no plugins, `Plugin::Rescan` → an empty scan,
 *   `Media::Preview/StopPreview` → Unit (after validating the source),
 *   `Recording::ListInputs` / `Engine::*` → fake devices.
 * - Validation covers the common invariants, not all of them (e.g. routing cycles through
 *   group outputs are not detected).
 *
 * ## Testing
 * `new MockTransport({ timers: "manual" })` disables real timers; drive time with
 * `tick(ms)` (wall-clock `modified_ms` then starts at a fixed date). `seed` makes ids and
 * meter noise deterministic.
 */

import { effectiveMute } from "@/domain/soloPlan";
import { contributions } from "@/ext";
import type {
  BrowseLocation,
  Command,
  DeviceId,
  EditCommand,
  EngineCommand,
  EngineStatus,
  EtherFile,
  Event,
  GestureId,
  HistoryState,
  MediaCommand,
  MediaRef,
  MediaSource,
  MeterFrame,
  PatchChange,
  PlayheadFrame,
  PluginCommand,
  Project,
  ProjectCommand,
  PluginDescriptor,
  ProjectId,
  ProjectSummary,
  RecordingCommand,
  ReplyValue,
  TrackId,
  TrackMeter,
  TransportCommand,
  TransportState,
  ForkOrigin,
} from "@/generated";
import { cmd } from "../cmd";
import { CommandFailedError, Emitter, type EngineTransport, type SendOptions, type Unsubscribe } from "../EngineTransport";
import { newId as defaultNewId } from "../ids";
import { createDemoProjects, createEmptyProject, createStarterProject } from "./demoProject";
import { fail, isDocumentCommand, isToggle, labelOf, reduceDocumentCommand } from "./documentReducer";
import { findLibraryFile, LIBRARY_ID, listLibraryFolder, MOCK_LOCATIONS, normalize, wavSize } from "./library";
import { synthesizePeaks } from "./peaks";
import { mockParamText } from "./plugins";
import { mulberry32, SEED_TIME, seededIdFactory } from "./random";
import { beatsToSeconds, bpmAt, signatureAt } from "./tempo";
import { changeKey, Tx } from "./tx";
import { MockExports } from "./roadmap/export";
import { MockPreview } from "./roadmap/mediaPreview";
import type { MockHost } from "./roadmap/host";
import { MockMidiLearn } from "./roadmap/midiLearn";
import { MockLiveRecord } from "./roadmap/liveRecord";
import { MockUploads } from "./roadmap/remote";
import { readMockMidiFile, writeMockMidiFile } from "./roadmap/midiFile";
// v0.2 runtime simulations, one file per node.
import { MockAnalysis } from "./roadmap/analysis";
import { MockBrowser } from "./roadmap/browserV2";
import { MockFreeze } from "./roadmap/freezeBounce";
import { libraryPath, MockMediaRefs } from "./roadmap/mediaReferences";
import { MOCK_INSTALLED_PLUGINS, MockResolver } from "./roadmap/resolver";
import { MockPresets } from "./roadmap/presets";
import { listModulatorKinds } from "./roadmap/racksModulation";
import { MockTimeEdits } from "./roadmap/timeEdits";
// v0.3: one file per node (`./roadmap/index.ts`).
import { audioToMidiCommand } from "./roadmap/audioToMidi";
import { MockCapture } from "./roadmap/capture";
import { externalCommand } from "./roadmap/external";
import { MockKeymap } from "./roadmap/keymap";
import { MockTemplates } from "./roadmap/templates";
import { MockUndoHistory } from "./roadmap/undoHistory";
// ai-chat: the agent API (Command::Agent) over the mock document.
import { MockAgent } from "./roadmap/agent";
import { MockVersions } from "./roadmap/versions";
import { detectBpm } from "./roadmap/shared";

/** `ether_protocol::recording::COMPUTER_KEYBOARD_PORT`: the port of `Recording::LiveMidi`. */
const COMPUTER_KEYBOARD_PORT = "Computer Keyboard";

export interface MockTransportOptions {
  /**
   * Initial content of the engine-side project store; the first one is opened on connect
   * (default: `createDemoProjects()`). Copied, never mutated.
   */
  projects?: Project[];
  /** Shorthand for `projects: [project]`. */
  project?: Project;
  /** Simulated round-trip latency per command, in ms (default 0 = next microtask). */
  latencyMs?: number;
  /** `"auto"` (default): real intervals; `"manual"`: time only advances via `tick(ms)`. */
  timers?: "auto" | "manual";
  /** Seed for engine-generated ids and meter noise (default: random ids, seed 1 noise). */
  seed?: number;
  /** Max undo steps (default 500). */
  historyLimit?: number;
}

/** File format tag / version of `.ether` files (see `ether-model/src/file.rs`). */
export const ETHER_FORMAT = "ethereal-project";
export const ETHER_VERSION = 1; // `CURRENT_VERSION` in ether-model/src/file.rs
const APP_VERSION = "0.0.1-mock";

const PLAYHEAD_INTERVAL_MS = 16;
const METER_INTERVAL_MS = 33;
const HOUR_MS = 3_600_000;
const UNIT: ReplyValue = { type: "Unit" };

interface HistoryEntry {
  /** undo-history: step id (monotonic per project, never reused). */
  id: number;
  /** undo-history: wall clock of the step's first commit. */
  time_ms: number;
  /** undo-history: checkpoint name. */
  checkpoint: string | null;
  label: string;
  gesture: GestureId | null;
  /** Final entity states (reapply to redo). */
  redo: PatchChange[];
  /** Initial entity states (reapply to undo). */
  undo: PatchChange[];
}

/** A project in the mock's engine-side store. */
interface StoredProject {
  /** The saved `.ether` document (JSON of an `EtherFile`). */
  json: string;
  name: string;
  modified_ms: number;
  /** The document's `settings.forked_from`. */
  forked_from?: ForkOrigin;
  /** Status (the controller's `project_status`): archived, in the bin since. */
  archived?: boolean;
  binned_ms?: number;
}

/** How long a project stays in the bin (`BIN_RETENTION_MS`). */
const BIN_RETENTION_MS = 30 * 24 * 3_600_000;

/** JSON round-trip: what a real host's serialization would do to the value. */
function wire<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function mergeChanges(a: PatchChange[], b: PatchChange[], keep: "first" | "last"): PatchChange[] {
  const map = new Map(a.map((c) => [changeKey(c), c]));
  for (const c of b) {
    const k = changeKey(c);
    if (keep === "last" || !map.has(k)) map.set(k, c);
  }
  return [...map.values()];
}

const dbToLinear = (db: number) => (db <= -144 ? 0 : Math.pow(10, db / 20));

export class MockTransport implements EngineTransport {
  readonly kind = "mock" as const;
  /** The fake installed plugins the import resolver matches against (`import-resolver/catalog.ts`). */
  installedPlugins(): PluginDescriptor[] {
    return MOCK_INSTALLED_PLUGINS;
  }

  private project: Project;
  /** Plugin devices held as placeholders (`OpenSafe`). */
  private safeMode: DeviceId[] = [];
  private safe = false;
  /** The open project the user discarded (`Project::Discard`): deleted once another opens. */
  private discarded: string | null = null;
  private revision = 0;
  private undoStack: HistoryEntry[] = [];
  private redoStack: HistoryEntry[] = [];
  private openGesture: GestureId | null = null;
  private nextStepId = 1;
  private historyTruncated = false;

  private readonly events = new Emitter<Event>();
  private readonly playheadEmitter = new Emitter<PlayheadFrame>();
  private readonly meterEmitter = new Emitter<MeterFrame>();

  private readonly latencyMs: number;
  private readonly manual: boolean;
  private readonly historyLimit: number;
  private readonly newId: () => string;
  private readonly rand: () => number;
  private queue: Promise<unknown> = Promise.resolve();
  private timers: ReturnType<typeof setInterval>[] = [];
  private disposed = false;

  // Clock.
  private manualNow = 0;
  private lastStepAt = 0;
  private meterAccumMs = 0;

  // Transport runtime.
  private playing = false;
  private position = 0;
  private startPosition = 0;
  private playheadDirty = true;
  private lastTransportJson = "";
  private tapTimes: number[] = [];

  // Engine-side project store + runtime state outside the document.
  private readonly store = new Map<ProjectId, StoredProject>();
  private dirty = false;
  private armed: TrackId[] = [];
  /** Punch in/out (runtime, like the controller's: not reset when a project opens). */
  private punch = false;

  // Meters runtime.
  private readonly levels = new Map<TrackId, number>();
  private metersSilent = false;

  // Roadmap v2 runtime simulations (`./roadmap`).
  private readonly host: MockHost = {
    project: () => this.project,
    emit: (event) => this.emit(event),
    newId: () => this.newId(),
    applyDocument: (commands, label) => void this.applyDocument(commands, label, null),
    execute: (command) => void this.execute(command, null),
    applyUntracked: (body) => {
      const tx = new Tx(this.project);
      try {
        body(tx);
      } catch (e) {
        tx.rollback();
        throw e;
      }
      if (!tx.isEmpty) this.emitPatch(tx.wireChanges());
    },
  };
  private readonly midiLearn = new MockMidiLearn(this.host);
  private readonly presets = new MockPresets(this.host);
  private readonly keymap = new MockKeymap(this.host);
  private readonly browser = new MockBrowser(this.host, (upload) => this.uploads.take(upload));
  private readonly exports = new MockExports(this.host);
  private readonly timeEdits = new MockTimeEdits({
    ...this.host,
    transact: (label, edit) =>
      void this.transact(label, null, true, (tx) => {
        edit(tx);
        return UNIT;
      }),
  });
  private readonly freeze = new MockFreeze({
    ...this.host,
    transact: (label, edit) =>
      void this.transact(label, null, true, (tx) => {
        const ctx = { tx, newId: this.newId, position: this.position };
        edit(tx, (c) => void reduceDocumentCommand(ctx, c));
        return UNIT;
      }),
  });
  /** undo-history: the history panel over the undo/redo stacks. */
  private readonly undoHistory = new MockUndoHistory({
    emit: (event) => this.emit(event),
    undoStack: () => this.undoStack,
    redoStack: () => this.redoStack,
    truncated: () => this.historyTruncated,
    move: (n) => this.moveHistory(n),
  });
  /** Missing media, relink, collect (`setOffline` for tests). */
  readonly mediaRefs = new MockMediaRefs({
    project: () => this.project,
    emit: (event) => this.emit(event),
    updateMedia: (label, media) =>
      void this.transact(label, null, true, (tx) => {
        for (const m of media) tx.upsert("Media", m);
        return UNIT;
      }),
    save: () => {
      if (this.discarded !== this.project.id) this.saveCurrent();
    },
    libraryHash: (rel) => hashHex(`library:${normalize(rel)}`),
  });
  /** Missing plugins / audio files (`roadmap/resolver.ts`). */
  readonly resolver = new MockResolver({
    project: () => this.project,
    emit: (event) => this.emit(event),
    newId: () => this.newId(),
    isMissing: (m) => this.mediaRefs.isMissing(m),
    libraryHash: (rel) => hashHex(`library:${normalize(rel)}`),
    commit: (label, devices, media) =>
      void this.transact(label, null, true, (tx) => {
        for (const d of devices) tx.upsert("Device", d);
        for (const m of media) tx.upsert("Media", m);
        return UNIT;
      }),
    edit: (label, f) =>
      void this.transact(label, null, true, (tx) => {
        f(tx);
        return UNIT;
      }),
  });
  /** v0.3 (`templates`): the template library (`roadmap/templates.ts`). */
  private readonly templates = new MockTemplates({
    ...this.host,
    now: () => this.wallNow(),
    newProject: (id, name, template) => {
      const checked = this.checkNewProject(id, name);
      this.saveBeforeSwitch();
      const project = template ? { ...template, id, settings: { ...template.settings, name: checked } } : createStarterProject(this.newId, checked, id);
      this.storeProject(project, this.wallNow());
      this.loadProject(project);
      this.emitListChanged();
      return { type: "Project", project: this.project };
    },
  });
  private readonly analysis = new MockAnalysis(this.host);
  private readonly agent = new MockAgent(this.host);
  private readonly preview = new MockPreview(this.host);
  private readonly uploads = new MockUploads((event) => this.emit(event));
  private readonly liveRecord = new MockLiveRecord({
    ...this.host,
    position: () => this.position,
    playing: () => this.playing,
    armed: () => this.armed,
    play: () => this.transportCommand({ type: "Play" }),
    commit: (media, commands) =>
      void this.transact("record", null, true, (tx) => {
        for (const m of media) tx.upsert("Media", m);
        const ctx = { tx, newId: this.newId, position: this.position };
        for (const c of commands) reduceDocumentCommand(ctx, c);
        return UNIT;
      }),
  });

  /** `keyboard-midi`: every `Recording::LiveMidi` message received, in order (tests). */
  readonly liveMidi: Array<[number, number, number]> = [];

  /** Every `Recording::PreviewNote` received, in order (tests). */
  readonly previewNotes: Array<{ track: string; data: [number, number, number] }> = [];

  /** `capture-midi`: the always-on MIDI capture buffer (fed by `simulateMidiInput`). */
  private readonly capture = new MockCapture({
    ...this.host,
    now: () => this.now(),
    position: () => this.position,
    playing: () => this.playing,
    commit: (commands, lanes) =>
      void this.transact("capture midi", null, true, (tx) => {
        const ctx = { tx, newId: this.newId, position: this.position };
        for (const c of commands) reduceDocumentCommand(ctx, c);
        for (const lane of lanes) tx.upsert("ExpressionLane", lane);
        return UNIT;
      }),
  });
  /** Rolling versions and crash recovery (`simulateCrash` for tests). */
  readonly versions = new MockVersions(
    {
      project: () => this.project,
      revision: () => this.revision,
      now: () => this.wallNow(),
      emit: (event) => this.emit(event),
      summaries: () => this.summaries(),
      savedJson: (id) => this.store.get(id)?.json,
      replaceDocument: (project) => {
        this.loadProject(project);
        this.setDirty(true);
      },
      saveIfDirty: () => {
        this.saveBeforeSwitch();
      },
    },
    parseEtherFile,
    serializeEtherFile,
  );

  constructor(opts: MockTransportOptions = {}) {
    this.manual = opts.timers === "manual";
    const initial = opts.projects ?? (opts.project ? [opts.project] : createDemoProjects());
    if (initial.length === 0) throw new Error("MockTransport needs at least one project");
    // Stored projects get staggered save times (first = most recent).
    initial.forEach((p, i) => this.storeProject(p, this.wallNow() - i * HOUR_MS));
    this.project = wire(initial[0]!);
    this.latencyMs = Math.max(0, opts.latencyMs ?? 0);
    this.historyLimit = opts.historyLimit ?? 500;
    this.newId = opts.seed !== undefined ? seededIdFactory(opts.seed + 1000) : defaultNewId;
    this.rand = mulberry32(opts.seed ?? 1);
    this.versions.projectLoaded();
    for (const d of contributions("mock.domain")) d.attach?.(this.ext);
  }

  /**
   * What the extensions' mock domains (`@/ext` "mock.domain") may use: the mock's host (the
   * roadmap mocks' interface), events, uploads and downloads, the stored projects.
   */
  readonly ext: MockExtensionHost = {
    transport: this,
    host: this.host,
    emit: (event) => this.emit(event),
    newId: () => this.newId(),
    takeUpload: (upload) => this.uploads.takeBytes(upload),
    addDownload: (d, bytes) => this.exports.addDownload(d, bytes),
    project: () => this.project,
    storedJson: (id) => this.stored(id).json,
    emptyProject: (id, name) => createEmptyProject(this.newId, name, id),
    storeAndOpen: (project) => {
      this.saveBeforeSwitch();
      const summary = this.storeProject(project, this.wallNow());
      this.loadProject(project);
      this.emitListChanged();
      return summary;
    },
  };

  /** An extension's answer for `command` (`mock.domain`), or `undefined`. */
  private extensionCommand(domain: string, command: unknown): ReplyValue | undefined {
    for (const d of contributions("mock.domain")) {
      if (d.domain !== domain) continue;
      const reply = d.handle(command, this.ext);
      if (reply !== undefined) return reply as ReplyValue;
    }
    return undefined;
  }

  // ─── EngineTransport ──────────────────────────────────────────────────────────────────

  connect(): Promise<Project> {
    if (this.disposed) return Promise.reject(new Error("MockTransport disposed"));
    this.lastStepAt = this.now();
    if (!this.manual && this.timers.length === 0) {
      this.timers.push(setInterval(() => this.step(), PLAYHEAD_INTERVAL_MS));
      this.timers.push(setInterval(() => this.meterStep(), METER_INTERVAL_MS));
    }
    this.syncTransport(true);
    this.emit({ type: "Recording", event: { type: "ArmChanged", armed: this.armed } });
    this.emitListChanged();
    this.emit({ type: "Project", event: { type: "DirtyChanged", dirty: this.dirty } });
    return Promise.resolve(wire(this.project));
  }

  send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    const run = async (): Promise<ReplyValue> => {
      if (this.latencyMs > 0) await new Promise((r) => setTimeout(r, this.latencyMs));
      if (this.disposed) throw new CommandFailedError({ code: "InvalidState", message: "transport disposed" }, command);
      try {
        return wire(this.execute(command, opts?.gesture ?? null));
      } catch (e) {
        if (e instanceof CommandFailedError) throw new CommandFailedError(e.error, command);
        throw new CommandFailedError({ code: "Internal", message: String(e) }, command);
      }
    };
    // Commands run strictly in order, like on a real connection.
    const result = this.queue.then(run, run);
    this.queue = result.catch(() => undefined);
    return result;
  }

  onEvent(listener: (event: Event) => void): Unsubscribe {
    return this.events.on(listener);
  }

  subscribePlayhead(listener: (frame: PlayheadFrame) => void): Unsubscribe {
    return this.playheadEmitter.on(listener);
  }

  subscribeMeters(listener: (frame: MeterFrame) => void): Unsubscribe {
    return this.meterEmitter.on(listener);
  }

  dispose(): void {
    this.disposed = true;
    for (const t of this.timers) clearInterval(t);
    this.timers = [];
    this.events.clear();
    this.playheadEmitter.clear();
    this.meterEmitter.clear();
  }

  // ─── Test / debug helpers ─────────────────────────────────────────────────────────────

  /**
   * Advance virtual time by `ms` (manual timers only), in ~16 ms steps: moves the
   * playhead and emits playhead/meter frames.
   */
  tick(ms: number): void {
    if (!this.manual) throw new Error('MockTransport.tick() requires { timers: "manual" }');
    let left = ms;
    while (left > 0) {
      const dt = Math.min(PLAYHEAD_INTERVAL_MS, left);
      left -= dt;
      this.manualNow += dt;
      this.step();
      this.meterAccumMs += dt;
      if (this.meterAccumMs >= METER_INTERVAL_MS) {
        this.meterAccumMs -= METER_INTERVAL_MS;
        this.meterStep();
      }
    }
  }

  /** A copy of the engine-side document. */
  snapshot(): Project {
    return wire(this.project);
  }

  get currentRevision(): number {
    return this.revision;
  }

  get playheadPosition(): number {
    return this.position;
  }

  /** Summaries of the engine-side project store, most recently saved first. */
  storedProjects(): ProjectSummary[] {
    return this.summaries();
  }

  // ─── Dispatch ─────────────────────────────────────────────────────────────────────────

  private execute(command: Command, gesture: GestureId | null): ReplyValue {
    const isEndGesture = command.domain === "Edit" && (command.command.type === "EndGesture" || command.command.type === "CancelGesture");
    // Upload chunks edit nothing: like the engine, they leave an open gesture open (an
    // upload-then-import-with-clip is one undo step).
    const isUpload = command.domain === "Media" && ["BeginUpload", "UploadChunk", "CancelUpload"].includes(command.command.type);
    // Live notes (computer keyboard, piano-roll preview during a note drag) edit nothing
    // either: the engine only moves the open gesture on a commit.
    const isLiveNote = command.domain === "Recording" && ["LiveMidi", "PreviewNote"].includes(command.command.type);
    if (!isEndGesture && !isUpload && !isLiveNote && gesture !== this.openGesture) this.openGesture = null;

    // Project commands first: `Rename` is a document edit only for the current project.
    if (command.domain === "Project") return this.projectCommand(command.command, gesture);
    if (isDocumentCommand(command)) return this.applyDocument([command], labelOf(command), gesture, !isToggle(command));

    if (command.domain === "Agent") return this.agent.command(command.command);
    const extension = this.extensionCommand(command.domain, command.command);
    if (extension !== undefined) return extension;

    switch (command.domain) {
      case "Transport":
        return this.transportCommand(command.command);
      case "Edit":
        return this.editCommand(command.command, gesture);
      case "Device":
        // Queries (ListBuiltin / GetDescriptor): run the reducer on a throwaway transaction.
        return reduceDocumentCommand({ tx: new Tx(this.project), newId: this.newId, position: this.position }, command);
      case "Plugin":
        return this.pluginCommand(command.command);
      case "Recording":
        return this.recordingCommand(command.command);
      case "Media":
        return this.mediaCommand(command.command, gesture);
      case "Engine":
        return this.engineCommand(command.command);
      case "Warp": {
        // `warp::detect_tempo` (the only non-document warp command).
        if (command.command.type !== "DetectTempo") return fail("Internal", "not a warp query");
        const cl = this.project.clips[command.command.clip] ?? fail("NotFound", `clip ${command.command.clip}`);
        if (cl.content.type !== "Audio") return fail("InvalidArgument", `clip ${cl.id} is not an audio clip`);
        const m = this.project.media[cl.content.media] ?? fail("NotFound", `media ${cl.content.media}`);
        return { type: "Tempo", bpm: detectBpm(m.frames / Math.max(1, m.sample_rate)) };
      }
      case "Export": {
        const reply = this.exports.command(command.command);
        this.syncTransport();
        return reply;
      }
      case "MidiMap":
        return this.midiLearn.command(command.command);
      // v0.2.
      case "Freeze":
        return this.freeze.command(command.command);
      case "TimeEdit":
        return this.timeEdits.command(command.command);
      case "Preset":
        return this.presets.command(command.command);
      case "Browser":
        return this.browser.command(command.command);
      case "Analysis":
        return this.analysis.command(command.command);
      case "MediaRef":
        return this.mediaRefs.command(command.command);
      case "Modulation":
        return listModulatorKinds();
      // v0.3. Document commands (`Expression::*`, `External::SetRouting`,
      // `Template::Insert`) went through `applyDocument` above.
      case "Capture":
        return this.capture.command(command.command);
      case "AudioToMidi":
        return audioToMidiCommand(command.command, this.host);
      case "External":
        if (command.command.type === "SetRouting") break;
        return externalCommand(command.command);
      case "History":
        // `State`: the undo/redo buttons' state (as patches carry it); doesn't subscribe.
        if (command.command.type === "State") return { type: "HistoryState", history: this.historyState() };
        return this.undoHistory.command(command.command);
      case "Template":
        if (command.command.type === "Insert") break;
        return this.templates.command(command.command);
      case "Version":
        return this.versions.command(command.command);
      case "Keymap":
        return this.keymap.command(command.command);
      // The mock names new things in English.
      case "Settings":
        return { type: "Unit" };
      case "Resolve":
        return this.resolver.command(command.command);
      default:
        return fail("InvalidArgument", `unknown command domain`);
    }
    return fail("InvalidArgument", `unknown command`);
  }

  /** Apply document commands as one transaction / undo step (none with `record: false`) and emit its patch. */
  private applyDocument(commands: Command[], label: string, gesture: GestureId | null, record = true): ReplyValue {
    const warnings: string[] = [];
    const value = this.transact(label, gesture, record, (tx) => {
      const ctx = { tx, newId: this.newId, position: this.position, warn: (m: string) => void warnings.push(m) };
      let value: ReplyValue = UNIT;
      for (const c of commands) value = reduceDocumentCommand(ctx, c);
      return value;
    });
    // Like the controller: warnings of a committed edit become notifications.
    for (const message of warnings) this.emit({ type: "Notification", level: "Warning", message });
    return value;
  }

  /** Run `body` as one atomic transaction / undo step (none with `record: false`) and emit its patch. */
  private transact(label: string, gesture: GestureId | null, record: boolean, body: (tx: Tx) => ReplyValue): ReplyValue {
    const tx = new Tx(this.project);
    let value: ReplyValue;
    try {
      value = body(tx);
    } catch (e) {
      tx.rollback();
      throw e;
    }
    if (!tx.isEmpty) {
      if (record) this.record(label, tx, gesture);
      this.emitPatch(tx.wireChanges());
    }
    this.syncTransport();
    return value;
  }

  // ─── History ──────────────────────────────────────────────────────────────────────────

  private record(label: string, tx: Tx, gesture: GestureId | null): void {
    const top = this.undoStack.at(-1);
    if (gesture !== null && this.openGesture === gesture && top?.gesture === gesture) {
      top.redo = mergeChanges(top.redo, tx.changes(), "last");
      top.undo = mergeChanges(top.undo, tx.inverse(), "first");
    } else {
      this.undoStack.push({
        id: this.nextStepId++,
        time_ms: this.wallNow(),
        checkpoint: null,
        label,
        gesture,
        redo: tx.changes(),
        undo: tx.inverse(),
      });
      if (this.undoStack.length > this.historyLimit) {
        this.undoStack.shift();
        this.historyTruncated = true;
      }
    }
    this.openGesture = gesture;
    this.redoStack = [];
    this.undoHistory.changed();
  }

  /** undo-history: undo (`n < 0`) or redo (`n > 0`) `|n|` steps as one patch. */
  private moveHistory(n: number): void {
    const tx = new Tx(this.project);
    for (let i = 0; i < Math.abs(n); i++) {
      const entry = n < 0 ? this.undoStack.pop() : this.redoStack.pop();
      if (!entry) break;
      (n < 0 ? this.redoStack : this.undoStack).push(entry);
      for (const c of n < 0 ? entry.undo : entry.redo) tx.write(c);
    }
    this.openGesture = null;
    this.emitPatch(tx.wireChanges());
    this.syncTransport();
    this.undoHistory.changed();
  }

  private historyState(): HistoryState {
    return {
      can_undo: this.undoStack.length > 0,
      can_redo: this.redoStack.length > 0,
      undo_label: this.undoStack.at(-1)?.label ?? null,
      redo_label: this.redoStack.at(-1)?.label ?? null,
      undo_id: this.undoStack.at(-1)?.id ?? null,
      redo_id: this.redoStack.at(-1)?.id ?? null,
    };
  }

  private replay(changes: PatchChange[]): void {
    const tx = new Tx(this.project);
    for (const c of changes) tx.write(c);
    this.emitPatch(tx.wireChanges());
    this.syncTransport();
  }

  private editCommand(c: EditCommand, gesture: GestureId | null): ReplyValue {
    switch (c.type) {
      case "Undo": {
        const entry = this.undoStack.pop() ?? fail("InvalidState", "nothing to undo");
        this.redoStack.push(entry);
        this.openGesture = null;
        this.replay(entry.undo);
        this.undoHistory.changed();
        return UNIT;
      }
      case "Redo": {
        const entry = this.redoStack.pop() ?? fail("InvalidState", "nothing to redo");
        this.undoStack.push(entry);
        this.openGesture = null;
        this.replay(entry.redo);
        this.undoHistory.changed();
        return UNIT;
      }
      case "EndGesture":
        if (this.openGesture === c.gesture) this.openGesture = null;
        return UNIT;
      case "CancelGesture": {
        // Take back the open gesture's step and forget it (nothing to redo).
        if (this.openGesture !== c.gesture) return UNIT;
        this.openGesture = null;
        const entry = this.undoStack.pop();
        if (entry) {
          this.replay(entry.undo);
          this.undoHistory.changed();
        }
        return UNIT;
      }
      case "Batch":
        for (const sub of c.commands) {
          if (!isDocumentCommand(sub) || sub.domain === "Transport") {
            fail("InvalidArgument", `${sub.domain}::${sub.command.type} is not allowed in a batch`);
          }
        }
        return this.applyDocument(c.commands, c.label, gesture);
    }
  }

  // ─── Events ───────────────────────────────────────────────────────────────────────────

  private emit(event: Event): void {
    this.events.emit(wire(event));
  }

  private emitPatch(changes: PatchChange[]): void {
    this.revision += 1;
    this.emit({ type: "Patch", patch: { revision: this.revision, changes, history: this.historyState() } });
    this.setDirty(true);
    // A relink (or its undo) may resolve or lose media.
    if (changes.some((c) => (c.type === "Upsert" && c.entity.type === "Media") || (c.type === "Remove" && c.key.type === "Media"))) {
      this.mediaRefs.mediaChanged();
    }
    // The current project's name is shown in the project list.
    if (changes.some((c) => c.type === "Settings")) this.emitListChanged();
    // Deleted tracks can't stay armed.
    const armed = this.armed.filter((id) => this.project.tracks[id]);
    if (armed.length !== this.armed.length) this.setArmed(armed);
  }

  private transportState(): TransportState {
    const s = this.project.settings;
    return {
      playing: this.playing,
      recording: this.liveRecord.recording,
      loop_enabled: s.loop_enabled,
      loop_region: s.loop_region,
      bpm: bpmAt(this.project, this.position),
      time_signature: signatureAt(this.project, this.position),
      metronome: s.metronome,
      start_position: this.startPosition,
      // An export runs: the live engine is paused and plays the render so far.
      ...(this.exports.rendering() ? { rendering: this.exports.rendering()! } : {}),
    };
  }

  /** Emit `Event::Transport` if any field changed (or `force`). */
  private syncTransport(force = false): void {
    this.capture.sync();
    const state = this.transportState();
    const json = JSON.stringify(state);
    if (!force && json === this.lastTransportJson) return;
    this.lastTransportJson = json;
    this.emit({ type: "Transport", state });
  }

  private loadProject(project: Project): void {
    this.liveRecord.abort();
    // A discarded project is deleted once another opens (as the controller).
    if (this.discarded && this.discarded !== project.id) {
      this.store.delete(this.discarded);
      this.discarded = null;
      this.emitListChanged();
    }
    this.project = project;
    this.safe = false;
    this.safeMode = [];
    this.undoStack = [];
    this.redoStack = [];
    this.openGesture = null;
    this.nextStepId = 1;
    this.historyTruncated = false;
    this.playing = false;
    this.position = 0;
    this.startPosition = 0;
    this.levels.clear();
    this.playheadDirty = true;
    this.emit({ type: "ProjectLoaded", project });
    this.undoHistory.changed();
    this.mediaRefs.projectOpened();
    this.versions.projectLoaded();
    this.setArmed([]);
    this.capture.projectChanged();
    this.setDirty(false);
    this.syncTransport();
  }

  private setDirty(dirty: boolean): void {
    if (dirty === this.dirty) return;
    this.dirty = dirty;
    this.emit({ type: "Project", event: { type: "DirtyChanged", dirty } });
  }

  private setArmed(armed: TrackId[]): void {
    if (armed.length === this.armed.length && armed.every((id, i) => id === this.armed[i])) return;
    this.armed = armed;
    this.emit({ type: "Recording", event: { type: "ArmChanged", armed } });
  }

  // ─── Project ──────────────────────────────────────────────────────────────────────────

  private wallNow(): number {
    return this.manual ? SEED_TIME + this.manualNow : Date.now();
  }

  private storeProject(project: Project, modified_ms: number): ProjectSummary {
    // A save keeps the status (a new project, a copy, has none).
    const { archived, binned_ms } = this.store.get(project.id) ?? {};
    const entry: StoredProject = {
      json: serializeEtherFile(project),
      name: project.settings.name,
      modified_ms,
      forked_from: project.settings.forked_from,
      archived,
      binned_ms,
    };
    this.store.set(project.id, entry);
    return this.summaryOf(project.id, entry, false);
  }

  /** `current`: the open project's (possibly unsaved) name is what the user sees. */
  private summaryOf(id: ProjectId, e: StoredProject, current = true): ProjectSummary {
    return {
      id,
      name: current && id === this.project.id ? this.project.settings.name : e.name,
      modified_ms: e.modified_ms,
      ...(e.forked_from ? { forked_from: e.forked_from } : {}),
      ...(e.archived ? { archived: true } : {}),
      ...(e.binned_ms !== undefined ? { binned_ms: e.binned_ms } : {}),
    };
  }

  /** The live projects (`binned`: the bin), the bin's expired projects deleted first. */
  private summaries(binned = false): ProjectSummary[] {
    const now = this.wallNow();
    for (const [id, e] of this.store) if (e.binned_ms !== undefined && now - e.binned_ms >= BIN_RETENTION_MS) this.store.delete(id);
    return [...this.store.entries()]
      .filter(([, e]) => (e.binned_ms !== undefined) === binned)
      .map(([id, e]) => this.summaryOf(id, e))
      .sort(binned ? (a, b) => (b.binned_ms ?? 0) - (a.binned_ms ?? 0) : (a, b) => b.modified_ms - a.modified_ms || a.name.localeCompare(b.name));
  }

  /** Change a stored project's status (`MoveToBin`, `RestoreFromBin`, `SetArchived`). */
  private setStatus(id: ProjectId, patch: Partial<Pick<StoredProject, "archived" | "binned_ms">>): ReplyValue {
    const entry = this.stored(id);
    this.store.set(id, { ...entry, ...patch });
    this.emitListChanged();
    return UNIT;
  }

  private emitListChanged(): void {
    this.emit({ type: "Project", event: { type: "ListChanged", projects: this.summaries() } });
  }

  private stored(id: ProjectId): StoredProject {
    return this.store.get(id) ?? fail("NotFound", `project ${id}`);
  }

  private checkNewProject(id: ProjectId, name: string): string {
    if (this.store.has(id) || id === this.project.id) fail("InvalidArgument", `project ${id} already exists`);
    const trimmed = name.trim();
    if (!trimmed) fail("InvalidArgument", "project name must not be empty");
    return trimmed;
  }

  /** Save the current project into the store. */
  /** Before another project opens: save unsaved changes (not a discarded project's). */
  private saveBeforeSwitch(): void {
    if (this.dirty && this.discarded !== this.project.id) this.saveCurrent();
  }

  private saveCurrent(): ProjectSummary {
    if (this.discarded === this.project.id) fail("InvalidState", "this project was discarded: it is deleted when it closes");
    const summary = this.storeProject(this.project, this.wallNow());
    this.emit({ type: "Project", event: { type: "Saved", project: summary } });
    this.emitListChanged();
    this.setDirty(false);
    return summary;
  }

  private projectCommand(c: ProjectCommand, gesture: GestureId | null): ReplyValue {
    switch (c.type) {
      case "List":
        return { type: "Projects", projects: this.summaries() };
      case "Get":
        return { type: "Project", project: this.project };
      case "Create": {
        const name = this.checkNewProject(c.id, c.name);
        this.saveBeforeSwitch();
        const project = createEmptyProject(this.newId, name, c.id);
        this.storeProject(project, this.wallNow());
        this.loadProject(project);
        this.emitListChanged();
        return { type: "Project", project: this.project };
      }
      case "Open":
      case "OpenSafe":
      case "OpenMirrored": {
        const entry = this.stored(c.id);
        if (entry.binned_ms !== undefined) fail("InvalidState", "this project is in the bin: restore it first");
        this.saveBeforeSwitch();
        // Re-read after a possible autosave (opening the current project reloads it).
        this.loadProject(parseEtherFile(this.store.get(c.id)?.json ?? entry.json));
        // Safe mode: the mock hosts no plugins; it reports their devices as held.
        if (c.type === "OpenSafe" || (c.type === "OpenMirrored" && c.safe)) {
          this.safeMode = Object.values(this.project.devices)
            .filter((d) => d.kind.type === "Plugin")
            .map((d) => d.id);
          this.safe = true;
          this.emit({ type: "Project", event: { type: "SafeMode", active: true, devices: this.safeMode } });
        }
        // The UI mirrors the document from `ProjectLoaded`: no second copy in the reply.
        if (c.type === "OpenMirrored") return UNIT;
        return { type: "Project", project: this.project };
      }
      case "LoadPlugins":
        if (this.safe) {
          this.safe = false;
          this.safeMode = [];
          this.emit({ type: "Project", event: { type: "SafeMode", active: false, devices: [] } });
        }
        return UNIT;
      case "Save":
        return { type: "Saved", project: this.saveCurrent() };
      case "SaveAs": {
        // As the controller: the open document takes the new id and name; playback,
        // the playhead, armed tracks and safe mode go on; only the history starts over.
        if (c.new_id === this.project.id) return { type: "Project", project: this.project };
        const name = this.checkNewProject(c.new_id, c.name);
        const copy: Project = { ...wire(this.project), id: c.new_id };
        // A fork, as the controller: where it came from.
        const forked_from = { project: this.project.id, name: this.project.settings.name, created_ms: this.wallNow() };
        copy.settings = { ...copy.settings, name, forked_from };
        const summary = this.storeProject(copy, this.wallNow());
        this.project = copy;
        this.undoStack = [];
        this.redoStack = [];
        this.openGesture = null;
        this.historyTruncated = false;
        this.emit({ type: "ProjectLoaded", project: copy });
        this.emit({ type: "Project", event: { type: "Saved", project: summary } });
        this.setDirty(false);
        if (this.safe) this.emit({ type: "Project", event: { type: "SafeMode", active: true, devices: this.safeMode } });
        this.undoHistory.changed();
        this.emitListChanged();
        return { type: "Project", project: this.project };
      }
      case "Duplicate": {
        const src = this.stored(c.id);
        const name = this.checkNewProject(c.new_id, c.name);
        // The open project is copied as it is now (unsaved changes included), as the controller.
        const copy: Project = { ...(c.id === this.project.id ? wire(this.project) : parseEtherFile(src.json)), id: c.new_id };
        const forked_from = { project: c.id, name: copy.settings.name, created_ms: this.wallNow() };
        copy.settings = { ...copy.settings, name, forked_from };
        const summary = this.storeProject(copy, this.wallNow());
        this.emitListChanged();
        return { type: "Saved", project: summary };
      }
      case "Rename": {
        if (c.id === this.project.id) {
          return this.applyDocument([{ domain: "Project", command: c }], "rename project", gesture);
        }
        const entry = this.stored(c.id);
        const name = c.name.trim();
        if (!name) fail("InvalidArgument", "project name must not be empty");
        const project = parseEtherFile(entry.json);
        project.settings = { ...project.settings, name };
        this.storeProject(project, this.wallNow());
        this.emitListChanged();
        return UNIT;
      }
      case "Discard":
        if (c.id === this.project.id) {
          // Never saved again; deleted once another project opens.
          this.discarded = c.id;
          this.setDirty(false);
          return UNIT;
        }
        this.stored(c.id);
        this.store.delete(c.id);
        this.emitListChanged();
        return UNIT;
      case "Delete":
        if (c.id === this.project.id) fail("InvalidState", "cannot delete the open project");
        this.stored(c.id);
        this.store.delete(c.id);
        this.emitListChanged();
        return UNIT;
      case "MoveToBin":
        if (c.id === this.project.id) fail("InvalidState", "cannot move the open project to the bin");
        return this.setStatus(c.id, { binned_ms: this.wallNow() });
      case "RestoreFromBin":
        return this.setStatus(c.id, { binned_ms: undefined });
      case "SetArchived":
        return this.setStatus(c.id, { archived: c.archived || undefined });
      case "ListBin":
        return { type: "Projects", projects: this.summaries(true) };
      case "EmptyBin":
        for (const p of this.summaries(true)) this.store.delete(p.id);
        this.emitListChanged();
        return UNIT;
      case "SetScale":
        // A document edit (applied by `documentReducer`).
        return this.applyDocument([{ domain: "Project", command: c }], "set scale", gesture);
      // The mock's bundle is the bare `.ether` document (no media, no archive;
      // the engine accepts both). OS paths are desktop-only.
      case "ExportBundle": {
        if (c.path !== null) fail("Unsupported", "bundles are delivered as downloads on this host");
        const json = c.id === this.project.id ? serializeEtherFile(this.project) : this.stored(c.id).json;
        const bytes = new TextEncoder().encode(json);
        const name =
          (parseEtherFile(json)
            .settings.name.replace(/[\\/:*?"<>|]/g, "_")
            .trim() || "Project") + ".ether";
        const download = { token: `bundle-${c.id}`, name, mime: "application/zip", size: bytes.length };
        this.exports.addDownload(download, bytes);
        return { type: "Bundle", download };
      }
      // The conversion needs the engine (`ether-export`).
      case "ExportTo":
        return fail("Unsupported", "exporting to other DAWs needs the engine");
      case "ImportBundle": {
        const existing = this.store.get(c.new_id);
        if (existing) return { type: "Saved", project: { id: c.new_id, name: existing.name, modified_ms: existing.modified_ms } };
        if (c.source.type === "Path") fail("Unsupported", "bundles are uploaded on this host");
        const bytes = this.uploads.takeBytes(c.source.upload);
        let project: Project;
        try {
          project = { ...parseEtherFile(new TextDecoder().decode(bytes)), id: c.new_id };
        } catch (e) {
          return fail("InvalidArgument", `this file is not a valid Ethereal project bundle (${e instanceof Error ? e.message : String(e)})`);
        }
        if (c.name !== null) project.settings = { ...project.settings, name: this.checkNewProject(c.new_id, c.name) };
        const summary = this.storeProject(project, this.wallNow());
        this.emitListChanged();
        return { type: "Saved", project: summary };
      }
      default:
        return this.extensionCommand("Project", c) ?? fail("Unsupported", `${c.type} is not available in this build`);
    }
  }

  // ─── Transport ────────────────────────────────────────────────────────────────────────

  private transportCommand(c: TransportCommand): ReplyValue {
    switch (c.type) {
      case "Play":
        this.startPlaying();
        break;
      case "Stop":
        if (this.playing) this.stopPlaying();
        else this.position = this.startPosition;
        break;
      case "TogglePlay":
        if (this.playing) this.stopPlaying();
        else this.startPlaying();
        break;
      case "Locate": {
        // A render runs: the playhead stays in its rendered part (as the controller).
        const r = this.exports.rendering();
        this.position = r ? Math.min(Math.max(c.position, r.start), Math.max(r.start, r.until)) : Math.max(0, c.position);
        // As the controller: the start position follows a locate only while stopped.
        if (!this.playing) this.startPosition = this.position;
        break;
      }
      case "TapTempo": {
        const now = this.now();
        this.tapTimes = [...this.tapTimes.filter((t) => now - t < 2000), now].slice(-4);
        if (this.tapTimes.length < 2) return { type: "Tempo", bpm: null };
        const first = this.tapTimes[0]!;
        const interval = (now - first) / (this.tapTimes.length - 1);
        const bpm = Math.round((60000 / interval) * 100) / 100;
        this.applyDocument([cmd("Transport", { type: "SetTempo", bpm })], "tap tempo", null);
        return { type: "Tempo", bpm: bpmAt(this.project, this.position) };
      }
      default:
        return fail("Internal", `unreachable: ${c.type} is a document command`);
    }
    this.playheadDirty = true;
    this.lastStepAt = this.now();
    this.syncTransport();
    this.emitPlayhead();
    return UNIT;
  }

  private startPlaying(): void {
    if (this.playing) return;
    // From where the playhead is (as the engine): a stop leaves it there.
    this.playing = true;
    this.lastStepAt = this.now();
    this.metersSilent = false;
  }

  private stopPlaying(): void {
    this.liveRecord.finish();
    this.playing = false;
  }

  // ─── Other domains ────────────────────────────────────────────────────────────────────

  private pluginCommand(c: PluginCommand): ReplyValue {
    switch (c.type) {
      case "List":
        return { type: "Plugins", plugins: [] };
      case "Rescan":
        this.emit({ type: "Plugin", event: { type: "ScanFinished", plugins: 0, failed: [] } });
        return UNIT;
      case "ListScanFailures":
        return { type: "ScanFailures", failed: [] };
      case "RetryScan":
        return fail("NotFound", `no failed plugin ${c.path}`);
      case "ParamTexts": {
        const d = this.project.devices[c.device] ?? fail("NotFound", `device ${c.device}`);
        if (d.kind.type !== "Plugin") return fail("InvalidArgument", `device ${c.device} is not a plugin`);
        const plugin = d.kind.plugin.plugin_id;
        return { type: "ParamTexts", texts: c.values.map((v) => mockParamText(plugin, v.param, v.value)) };
      }
      default:
        return fail("Unsupported", "plugins are not available in the mock engine");
    }
  }

  private recordingCommand(c: RecordingCommand): ReplyValue {
    if (c.type === "Arm") {
      const t = this.project.tracks[c.track] ?? fail("NotFound", `track ${c.track}`);
      if (t.kind !== "Audio" && t.kind !== "Midi") fail("InvalidArgument", `${t.kind} tracks can't be armed`);
      const others = c.exclusive ? [] : this.armed.filter((id) => id !== t.id);
      this.setArmed(c.armed ? [...others, t.id] : c.exclusive ? this.armed.filter((id) => id !== t.id) : others);
      return UNIT;
    }
    if (c.type === "SetPunch") {
      if (this.punch !== c.enabled) {
        this.punch = c.enabled;
        this.emit({ type: "Recording", event: { type: "PunchChanged", enabled: c.enabled } });
      }
      return UNIT;
    }
    if (c.type === "ListInputs") {
      return {
        type: "Inputs",
        inputs: {
          // A 4-channel interface, numbered like the desktop host's inputs.
          audio: [0, 1, 2, 3].map((index) => ({ index, name: `${index + 1}` })),
          midi: [{ id: "mock-midi", name: "Mock MIDI Keyboard" }],
        },
      };
    }
    if (c.type === "SetRecording") {
      if (c.enabled && this.exports.rendering()) fail("InvalidState", "a render is running: record is available once it ends");
      const reply = this.liveRecord.set(c.enabled);
      this.syncTransport();
      return reply;
    }
    if (c.type === "LiveMidi") {
      // `keyboard-midi`: the computer MIDI keyboard. Logged (`liveMidi`) for tests and fed to
      // MIDI capture like the controller does (not to MIDI learn: the controller's mapping
      // input is the hardware ports' queue).
      const [status, d1, d2] = c.data;
      if (status < 0x80 || status >= 0xf0 || d1 > 0x7f || d2 > 0x7f) fail("InvalidArgument", "not a channel voice message");
      this.liveMidi.push([status, d1, d2]);
      this.capture.input(COMPUTER_KEYBOARD_PORT, [status, d1, d2]);
      return UNIT;
    }
    if (c.type === "PreviewNote") {
      // Piano-roll note preview. Logged for tests; never captured or recorded.
      const [status, d1, d2] = c.data;
      if (status < 0x80 || status >= 0xf0 || d1 > 0x7f || d2 > 0x7f) fail("InvalidArgument", "not a channel voice message");
      if (!this.project.tracks[c.track]) fail("NotFound", `track ${c.track}`);
      this.previewNotes.push({ track: c.track, data: [status, d1, d2] });
      return UNIT;
    }
    return fail("Unsupported", "recording is not available in the mock engine");
  }

  private mediaCommand(c: MediaCommand, gesture: GestureId | null): ReplyValue {
    switch (c.type) {
      case "GetPeaks": {
        const media = this.project.media[c.request.media] ?? fail("NotFound", `media ${c.request.media}`);
        return { type: "Peaks", peaks: synthesizePeaks(media, c.request) };
      }
      case "ListLocations":
        return { type: "Locations", locations: [...MOCK_LOCATIONS, ...this.browser.folderLocations()] };
      case "ListDirectory":
        return {
          type: "Directory",
          listing: {
            location: c.location,
            path: normalize(c.path),
            // The user's file filter.
            entries: this.listDirectory(c.location, c.path).filter((e) =>
              e.kind === "Directory" ? this.browser.fileFilter.allowsDir(e.name) : this.browser.fileFilter.allowsFile(e.name),
            ),
          },
        };
      case "Import": {
        if (this.project.media[c.id]) fail("InvalidArgument", `media ${c.id} already exists`);
        const media = this.resolveImport(c.id, c.source);
        const value = this.transact("import audio", gesture, true, (tx) => {
          tx.upsert("Media", media);
          return { type: "Media", media };
        });
        this.emit({ type: "Media", event: { type: "PeaksReady", media: media.id } });
        return value;
      }
      case "Preview":
        this.checkSource(c.source);
        return this.preview.play(c.source);
      case "StopPreview":
        return this.preview.stop();
      case "BeginUpload":
      case "UploadChunk":
      case "CancelUpload":
        return this.uploads.command(c);
      // MIDI files (no SMF codec here: see roadmap/midiFile.ts).
      case "ReadMidiFile": {
        const s = c.source;
        if (s.type === "Location" && s.location.type === "Library" && s.location.id === LIBRARY_ID && !findLibraryFile(s.path))
          fail("NotFound", `file ${s.path}`);
        const file = readMockMidiFile(s, (upload) => {
          const name = this.uploads.check(upload).name;
          this.uploads.takeBytes(upload);
          return name;
        });
        return { type: "MidiFile", file };
      }
      case "WriteMidiFile":
        return { type: "MidiFileWritten", result: writeMockMidiFile(this.project, c.request) };
      case "Reveal":
        return fail("Unsupported", "showing files is not available in the mock engine");
    }
  }

  private listDirectory(location: BrowseLocation, path: string) {
    if (location.type === "Library") {
      if (location.id !== LIBRARY_ID) return this.browser.listFolder(location.id, path) ?? fail("NotFound", `location ${location.id}`);
      return listLibraryFolder(path) ?? fail("NotFound", `folder ${path}`);
    }
    // The project's media/ folder is flat.
    if (normalize(path) !== "") fail("NotFound", `folder ${path}`);
    const byFile = new Map<string, MediaRef>();
    for (const m of Object.values(this.project.media)) byFile.set(m.file, m);
    return [...byFile.values()]
      .map((m) => {
        const rel = m.file.replace(/^media\//, "");
        return { name: rel, path: rel, kind: "Audio" as const, size: wavSize(m.frames, m.channels) };
      })
      .sort((a, b) => a.name.localeCompare(b.name));
  }

  /** Validate a media source; returns the project media it refers to, if any. */
  private checkSource(source: MediaSource): MediaRef | null {
    switch (source.type) {
      case "Upload":
        this.uploads.check(source.upload);
        return null;
      case "Path":
        // v0.2 (`file-import`): OS paths exist only on the desktop engine.
        return fail("Unsupported", "importing OS files by path needs the desktop engine");
      case "Project":
        return this.project.media[source.media] ?? fail("NotFound", `media ${source.media}`);
      case "Location":
        if (source.location.type === "Library") {
          if (source.location.id !== LIBRARY_ID) fail("NotFound", `location ${source.location.id}`);
          const f = findLibraryFile(source.path) ?? fail("NotFound", `file ${source.path}`);
          if (f.kind !== "Audio") fail("Decode", `${source.path} is not an audio file`);
          return null;
        }
        return Object.values(this.project.media).find((m) => m.file === `media/${normalize(source.path)}`) ?? fail("NotFound", `file ${source.path}`);
    }
  }

  /** The `MediaRef` an import of `source` creates ("copying" the file into `media/`). */
  private resolveImport(id: string, source: MediaSource): MediaRef {
    if (source.type === "Project") fail("InvalidArgument", "media is already in the project");
    if (source.type === "Upload") {
      // `file-import`: a completed upload, copied into the project.
      const u = this.uploads.take(source.upload);
      const dup = Object.values(this.project.media).find((m) => m.hash === u.hash);
      return { ...u, id, file: dup?.file ?? `media/${id}-${u.name}`, location: { type: "Project" } };
    }
    const existing = this.checkSource(source);
    if (existing) return { ...existing, id };
    const path = normalize((source as { path: string }).path);
    const f = findLibraryFile(path)!;
    const name = path.slice(path.lastIndexOf("/") + 1);
    const hash = hashHex(`library:${path}`);
    return {
      id,
      name,
      file: `media/${id}-${name}`,
      sample_rate: f.sample_rate,
      channels: f.channels,
      frames: f.frames,
      hash,
      // Library files are referenced in place (like the desktop).
      location: { type: "External", path: libraryPath(path) },
    };
  }

  private engineCommand(c: EngineCommand): ReplyValue {
    const current = { backend: "mock", host: null, output_device: "Mock Output", input_device: "Mock Input", sample_rate: 48000, buffer_size: 256 };
    switch (c.type) {
      case "GetStatus":
        return { type: "Status", status: this.engineStatus() };
      case "SetAnticipative": {
        this.anticipative = c.on;
        const status = this.engineStatus();
        this.emit({ type: "Engine", event: { type: "Status", status } });
        return { type: "Status", status };
      }
      case "ListAudioDevices":
        return {
          type: "AudioDevices",
          devices: {
            backends: ["mock"],
            hosts: [],
            outputs: [{ name: "Mock Output", channels: 2, sample_rates: [44100, 48000], is_default: true, buffer_size_range: [32, 2048] }],
            inputs: [{ name: "Mock Input", channels: 2, sample_rates: [44100, 48000], is_default: true, buffer_size_range: null }],
            current,
            buffer_size_settable: true,
            hidden_devices: 0,
          },
        };
      case "SetAudioConfig":
        return fail("Unsupported", "the mock engine has no audio devices");
      // The mock has no graph to tear down: it only says the new state.
      case "SetRunning":
        if (this.engineRunning !== c.running) {
          this.engineRunning = c.running;
          this.emit({ type: "Engine", event: { type: "Running", running: c.running } });
        }
        return { type: "Unit" };
    }
  }

  /** `EngineCommand::SetAnticipative`'s state (on, like a desktop engine with workers). */
  private anticipative = true;

  private engineStatus(): EngineStatus {
    return {
      running: true,
      backend: "mock",
      sample_rate: 48000,
      buffer_size: 256,
      output_latency: 256,
      input_latency: 256,
      xruns: 0,
      instance: "mock",
      device_released: false,
      anticipative: this.anticipative,
    };
  }

  /** `EngineCommand::SetRunning`'s state. */
  private engineRunning = true;

  /**
   * Test/debug helper: feed one incoming MIDI short message, as a host would through
   * `EngineBridge::poll_midi_input`. Completes a pending learn, else drives the mapped target
   * (see `roadmap/midiLearn.ts`).
   */
  simulateMidiInput(port: string, data: [number, number, number]): void {
    // v0.3 (`capture-midi`): every message also feeds the capture buffer.
    this.capture.input(port, data);
    this.midiLearn.input(port, data);
  }

  // ─── Time ─────────────────────────────────────────────────────────────────────────────

  private now(): number {
    return this.manual ? this.manualNow : performance.now();
  }

  /** Advance the playhead to `now()` and publish a playhead frame. */
  private step(): void {
    const t = this.now();
    const dtMs = t - this.lastStepAt;
    this.lastStepAt = t;
    if (this.playing && dtMs > 0) {
      const beats = (dtMs / 1000) * (bpmAt(this.project, this.position) / 60);
      const { loop_enabled, loop_region } = this.project.settings;
      let next = this.position + beats;
      const len = loop_region.end - loop_region.start;
      if (loop_enabled && len > 0 && this.position < loop_region.end && next >= loop_region.end) {
        next = loop_region.start + ((next - loop_region.end) % len);
      }
      this.position = next;
      this.syncTransport(); // tempo/signature at the playhead may change
    }
    if (this.playing || this.playheadDirty) this.emitPlayhead();
    // A running export advanced: its `rendering` state moved.
    if (this.exports.step()) this.syncTransport();
    this.freeze.step();
    this.preview.step();
    this.liveRecord.step();
    this.versions.step();
  }

  private emitPlayhead(): void {
    this.playheadDirty = false;
    this.playheadEmitter.emit({
      transport: {
        position: this.position,
        seconds: beatsToSeconds(this.project, this.position),
        playing: this.playing,
        bpm: bpmAt(this.project, this.position),
      },
    });
  }

  /** How much signal a track "would" produce right now, 0..1 before its fader. */
  private activity(track: TrackId): number {
    if (!this.playing) return 0;
    for (const c of Object.values(this.project.clips)) {
      if (c.track !== track || c.muted) continue;
      if (this.position >= c.start && this.position < c.start + c.length) return 0.55;
    }
    return 0;
  }

  /** Synthesize one meter frame (peak-hold with decay, beat pulse, a bit of noise). */
  private meterStep(): void {
    this.analysis.step();
    const tracks = Object.values(this.project.tracks);
    const pulse = 0.7 + 0.3 * (1 - (this.position % 1));
    const decay = 0.82;
    const master = tracks.find((t) => t.kind === "Master");

    // Signal each track produces itself (clips), then route post-fader levels:
    // `Default` → parent group (or master at top level), `Track` → that track, sends → returns.
    const own = new Map<TrackId, number>();
    for (const t of tracks) own.set(t.id, this.activity(t.id) * pulse * (0.9 + 0.2 * this.rand()));
    const destination = (t: (typeof tracks)[number]): TrackId | null => {
      if (t.kind === "Master") return null;
      if (t.output.type === "Track") return t.output.track;
      if (t.output.type === "None") return null;
      return t.parent ?? master?.id ?? null;
    };
    const post = new Map<TrackId, number>();
    const visiting = new Set<TrackId>();
    const postOf = (id: TrackId): number => {
      const cached = post.get(id);
      if (cached !== undefined) return cached;
      const t = this.project.tracks[id];
      if (!t || visiting.has(id)) return 0; // routing cycle: ignore
      visiting.add(id);
      let input = own.get(id) ?? 0;
      for (const k of tracks) if (k.id !== id && destination(k) === id) input += postOf(k.id) * (t.kind === "Master" ? 0.6 : 0.7);
      for (const snd of Object.values(this.project.sends)) {
        if (snd.to === id) input += postOf(snd.from) * dbToLinear(snd.level) * 0.8;
      }
      visiting.delete(id);
      // A soloed track is heard even if muted (CONTRACTS.md §12.10).
      const level = effectiveMute(t.mixer.mute, t.mixer.solo) ? 0 : Math.min(1.2, input) * dbToLinear(t.mixer.volume);
      post.set(id, level);
      return level;
    };

    const meters: TrackMeter[] = [];
    let silent = true;
    for (const t of tracks) {
      const level = Math.max(postOf(t.id), (this.levels.get(t.id) ?? 0) * decay);
      const v = level < 0.001 ? 0 : level;
      this.levels.set(t.id, v);
      if (v > 0) silent = false;
      const left = v * Math.min(1, 1 - t.mixer.pan);
      const right = v * Math.min(1, 1 + t.mixer.pan);
      meters.push({ track: t.id, peak: [left, right], rms: [left * 0.7, right * 0.7], clipped: left >= 1 || right >= 1 });
    }
    // Once everything has decayed to silence, stop sending identical all-zero frames.
    if (silent && this.metersSilent) return;
    this.metersSilent = silent;
    this.meterEmitter.emit({ tracks: meters, cpu_load: this.playing ? 0.1 + 0.05 * this.rand() : 0.02 });
  }
}

/** Hex FNV-1a hash of a string (fake content hash). */
function hashHex(s: string): string {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return (h >>> 0).toString(16).padStart(8, "0");
}

/** Serialize a project as an `.ether` document (what the engine-side store writes). */
export function serializeEtherFile(project: Project): string {
  const file: EtherFile = { format: ETHER_FORMAT, format_version: ETHER_VERSION, app_version: APP_VERSION, project };
  return JSON.stringify(file);
}

/** Parse and minimally validate an `.ether` JSON document (engine-side store reads). */
export function parseEtherFile(json: string): Project {
  let file: Partial<EtherFile>;
  try {
    file = JSON.parse(json) as Partial<EtherFile>;
  } catch (e) {
    return fail("Decode", `invalid JSON: ${String(e)}`);
  }
  if (file.format !== ETHER_FORMAT) return fail("Decode", `not an Ethereal project (format: ${String(file.format)})`);
  if (typeof file.format_version !== "number" || file.format_version > ETHER_VERSION) {
    return fail("Decode", `unsupported project format version ${String(file.format_version)}`);
  }
  const p = file.project;
  const tables = [
    "tracks",
    "clips",
    "notes",
    "devices",
    "sends",
    "automation_lanes",
    "automation_points",
    "tempo_points",
    "time_signatures",
    "warp_markers",
    "media",
    "markers",
    "midi_mappings",
    "drum_pads",
  ] as const;
  if (!p || typeof p !== "object" || !p.settings || tables.some((t) => typeof p[t] !== "object" || p[t] === null)) {
    return fail("Decode", "malformed project");
  }
  if (Object.values(p.tracks).filter((t) => t.kind === "Master").length !== 1) return fail("Decode", "project must have exactly one master track");
  return p;
}

/** What an extension's mock domain gets (`MockTransport.ext`). */
export interface MockExtensionHost {
  transport: MockTransport;
  host: MockHost;
  emit(event: Event): void;
  newId(): string;
  takeUpload(upload: string): Uint8Array;
  addDownload(download: { token: string; name: string; mime: string; size: number }, bytes: Uint8Array): void;
  project(): Project;
  storedJson(id: ProjectId): string;
  emptyProject(id: ProjectId, name: string): Project;
  storeAndOpen(project: Project): ProjectSummary;
}
