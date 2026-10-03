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
 *   (`MediaLocation::External`, `media-references`; uploads are "copied" into the project:
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
 * ## Roadmap v2 (contracts-2) simulations (one file per feature in `./roadmap/`)
 * - Undoable document commands: `Tempo::*` (tempo/signature CRUD; the points at beat 0
 *   can't be removed or moved; metronome settings), `Marker::*`, `Clip::{SetFadeCurves,
 *   SetReversed, Crossfade}` (the crossfade just extends the first clip), `Device::
 *   SetSidechain`, `Groove::{Humanize (seeded, deterministic), SetSwing}`, quantize swing,
 *   `DrumRack::*` (pads + pad chains), `Slice::*` (auto slicing: `Equal`/`Grid`;
 *   `Transients` = 8 equal slices; `ToDrumRack` → `Unsupported`), `MidiMap::{Map, Edit,
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
 *   `Collab::*`, `Chat::*` and `PinnedNote::*` (base-62 stubs), `Slice::ToDrumRack`,
 *   `Warp::DetectTempo`, `Engine::SetAudioConfig`.
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

import type {
  BrowseLocation,
  Command,
  EditCommand,
  EngineCommand,
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
  ProjectId,
  ProjectSummary,
  RecordingCommand,
  ReplyValue,
  TrackId,
  TrackMeter,
  TransportCommand,
  TransportState,
} from "@/generated";
import { cmd } from "../cmd";
import { CommandFailedError, Emitter, type EngineTransport, type SendOptions, type Unsubscribe } from "../EngineTransport";
import { newId as defaultNewId } from "../ids";
import { createDemoProjects, createEmptyProject } from "./demoProject";
import { fail, isDocumentCommand, labelOf, reduceDocumentCommand } from "./documentReducer";
import { findLibraryFile, LIBRARY_ID, listLibraryFolder, MOCK_LOCATIONS, normalize, wavSize } from "./library";
import { synthesizePeaks } from "./peaks";
import { mulberry32, SEED_TIME, seededIdFactory } from "./random";
import { beatsToSeconds, bpmAt, signatureAt } from "./tempo";
import { changeKey, Tx } from "./tx";
import { MockCollab } from "./roadmap/collab";
import { MockShare } from "./roadmap/share";
import { MockExports } from "./roadmap/export";
import { MockPreview } from "./roadmap/mediaPreview";
import type { MockHost } from "./roadmap/host";
import { MockMidiLearn } from "./roadmap/midiLearn";
import { MockLiveRecord } from "./roadmap/liveRecord";
import { MockUploads } from "./roadmap/remote";
// v0.2 (contracts-3) runtime simulations, one file per node.
import { MockAnalysis } from "./roadmap/analysis";
import { MockBrowser } from "./roadmap/browserV2";
import { MockFreeze } from "./roadmap/freezeBounce";
import { libraryPath, MockMediaRefs } from "./roadmap/mediaReferences";
import { MockPresets } from "./roadmap/presets";
import { listModulatorKinds } from "./roadmap/racksModulation";
import { MockTimeEdits } from "./roadmap/timeEdits";
import { chatCommand } from "./roadmap/social";
// v0.3 (contracts-4): one file per node (`./roadmap/index.ts`).
import { audioToMidiCommand } from "./roadmap/audioToMidi";
import { MockCapture } from "./roadmap/capture";
import { externalCommand } from "./roadmap/external";
import { keymapCommand } from "./roadmap/keymap";
import { templateCommand } from "./roadmap/templates";
import { historyCommand } from "./roadmap/undoHistory";
// ai-chat: the agent API (Command::Agent) over the mock document.
import { MockAgent, type MockAgentCommand } from "./roadmap/agent";
import { MockVersions } from "./roadmap/versions";

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
export const ETHER_VERSION = 3;
const APP_VERSION = "0.0.1-mock";

const PLAYHEAD_INTERVAL_MS = 16;
const METER_INTERVAL_MS = 33;
const HOUR_MS = 3_600_000;
const UNIT: ReplyValue = { type: "Unit" };

interface HistoryEntry {
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
}

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

  private project: Project;
  private revision = 0;
  private undoStack: HistoryEntry[] = [];
  private redoStack: HistoryEntry[] = [];
  private openGesture: GestureId | null = null;

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
      if (!tx.isEmpty) this.emitPatch(tx.changes());
    },
  };
  private readonly midiLearn = new MockMidiLearn(this.host);
  private readonly presets = new MockPresets(this.host);
  private readonly browser = new MockBrowser(this.host);
  private readonly exports = new MockExports(this.host);
  private readonly timeEdits = new MockTimeEdits({
    ...this.host,
    transact: (label, edit) =>
      void this.transact(label, null, (tx) => {
        edit(tx);
        return UNIT;
      }),
  });
  private readonly freeze = new MockFreeze({
    ...this.host,
    transact: (label, edit) =>
      void this.transact(label, null, (tx) => {
        const ctx = { tx, newId: this.newId, position: this.position };
        edit(tx, (c) => void reduceDocumentCommand(ctx, c));
        return UNIT;
      }),
  });
  private readonly collab = new MockCollab(this.host);
  /** base-115 sharing simulation (docs/SHARING.md; `simulateJoin`, `simulateHostOnline`). */
  readonly share = new MockShare(this.host);
  /** `media-references`: missing media, relink, collect (`setOffline` for tests). */
  readonly mediaRefs = new MockMediaRefs({
    project: () => this.project,
    emit: (event) => this.emit(event),
    updateMedia: (label, media) =>
      void this.transact(label, null, (tx) => {
        for (const m of media) tx.upsert("Media", m);
        return UNIT;
      }),
    save: () => void this.saveCurrent(),
    libraryHash: (rel) => hashHex(`library:${normalize(rel)}`),
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
      void this.transact("Record", null, (tx) => {
        for (const m of media) tx.upsert("Media", m);
        const ctx = { tx, newId: this.newId, position: this.position };
        for (const c of commands) reduceDocumentCommand(ctx, c);
        return UNIT;
      }),
  });

  /** `capture-midi`: the always-on MIDI capture buffer (fed by `simulateMidiInput`). */
  private readonly capture = new MockCapture({
    ...this.host,
    now: () => this.now(),
    position: () => this.position,
    playing: () => this.playing,
    commit: (commands, lanes) =>
      void this.transact("Capture", null, (tx) => {
        const ctx = { tx, newId: this.newId, position: this.position };
        for (const c of commands) reduceDocumentCommand(ctx, c);
        for (const lane of lanes) tx.upsert("ExpressionLane", lane);
        return UNIT;
      }),
  });
  /** `project-versions`: rolling versions and crash recovery (`simulateCrash` for tests). */
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
        if (this.dirty) this.saveCurrent();
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
    const isEndGesture = command.domain === "Edit" && command.command.type === "EndGesture";
    // Upload chunks edit nothing: like the engine, they leave an open gesture open (an
    // upload-then-import-with-clip is one undo step).
    const isUpload = command.domain === "Media" && ["BeginUpload", "UploadChunk", "CancelUpload"].includes(command.command.type);
    if (!isEndGesture && !isUpload && gesture !== this.openGesture) this.openGesture = null;

    // Project commands first: `Rename` is a document edit only for the current project.
    if (command.domain === "Project") return this.projectCommand(command.command, gesture);
    if (isDocumentCommand(command)) return this.applyDocument([command], labelOf(command), gesture);

    // ai-chat: `Command::Agent` (not in the generated `Command` until agent-api lands).
    const agent = command as unknown as { domain: string; command: MockAgentCommand };
    if (agent.domain === "Agent") return this.agent.command(agent.command);

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
      case "Warp":
        return fail("Unsupported", "tempo detection is not available in the mock engine");
      case "Export":
        return this.exports.command(command.command);
      case "MidiMap":
        return this.midiLearn.command(command.command);
      case "Collab":
        return this.collab.command(command.command);
      // v0.2 (contracts-3).
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
      case "Chat":
        return chatCommand(command.command, this.collab);
      // v0.3 (contracts-4). Document commands (`Expression::*`, `External::SetRouting`,
      // `Template::Insert`) went through `applyDocument` above.
      case "Capture":
        return this.capture.command(command.command);
      case "AudioToMidi":
        return audioToMidiCommand(command.command);
      case "External":
        if (command.command.type === "SetRouting") break;
        return externalCommand(command.command);
      case "History":
        return historyCommand(command.command);
      case "Template":
        if (command.command.type === "Insert") break;
        return templateCommand(command.command);
      case "Version":
        return this.versions.command(command.command);
      case "Keymap":
        return keymapCommand(command.command);
      // base-115 (docs/SHARING.md).
      case "Share":
        return this.share.command(command.command);
      default:
        return fail("InvalidArgument", `unknown command domain`);
    }
    return fail("InvalidArgument", `unknown command`);
  }

  /** Apply document commands as one transaction / undo step and emit its patch. */
  private applyDocument(commands: Command[], label: string, gesture: GestureId | null): ReplyValue {
    return this.transact(label, gesture, (tx) => {
      const ctx = { tx, newId: this.newId, position: this.position };
      let value: ReplyValue = UNIT;
      for (const c of commands) value = reduceDocumentCommand(ctx, c);
      return value;
    });
  }

  /** Run `body` as one atomic transaction / undo step and emit its patch. */
  private transact(label: string, gesture: GestureId | null, body: (tx: Tx) => ReplyValue): ReplyValue {
    const tx = new Tx(this.project);
    let value: ReplyValue;
    try {
      value = body(tx);
    } catch (e) {
      tx.rollback();
      throw e;
    }
    if (!tx.isEmpty) {
      this.record(label, tx, gesture);
      this.emitPatch(tx.changes());
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
      this.undoStack.push({ label, gesture, redo: tx.changes(), undo: tx.inverse() });
      if (this.undoStack.length > this.historyLimit) this.undoStack.shift();
    }
    this.openGesture = gesture;
    this.redoStack = [];
  }

  private historyState(): HistoryState {
    return {
      can_undo: this.undoStack.length > 0,
      can_redo: this.redoStack.length > 0,
      undo_label: this.undoStack.at(-1)?.label ?? null,
      redo_label: this.redoStack.at(-1)?.label ?? null,
    };
  }

  private replay(changes: PatchChange[]): void {
    const tx = new Tx(this.project);
    for (const c of changes) tx.write(c);
    this.emitPatch(tx.changes());
    this.syncTransport();
  }

  private editCommand(c: EditCommand, gesture: GestureId | null): ReplyValue {
    switch (c.type) {
      case "Undo": {
        const entry = this.undoStack.pop() ?? fail("InvalidState", "nothing to undo");
        this.redoStack.push(entry);
        this.openGesture = null;
        this.replay(entry.undo);
        return UNIT;
      }
      case "Redo": {
        const entry = this.redoStack.pop() ?? fail("InvalidState", "nothing to redo");
        this.undoStack.push(entry);
        this.openGesture = null;
        this.replay(entry.redo);
        return UNIT;
      }
      case "EndGesture":
        if (this.openGesture === c.gesture) this.openGesture = null;
        return UNIT;
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
    // `media-references`: a relink (or its undo) may resolve or lose media.
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
    this.project = project;
    this.undoStack = [];
    this.redoStack = [];
    this.openGesture = null;
    this.playing = false;
    this.position = 0;
    this.startPosition = 0;
    this.levels.clear();
    this.playheadDirty = true;
    this.emit({ type: "ProjectLoaded", project });
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
    const entry = { json: serializeEtherFile(project), name: project.settings.name, modified_ms };
    this.store.set(project.id, entry);
    return { id: project.id, name: entry.name, modified_ms };
  }

  private summaries(): ProjectSummary[] {
    return [...this.store.entries()]
      .map(([id, e]) => ({
        id,
        // The current project's (possibly unsaved) name is what the user sees.
        name: id === this.project.id ? this.project.settings.name : e.name,
        modified_ms: e.modified_ms,
      }))
      .sort((a, b) => b.modified_ms - a.modified_ms || a.name.localeCompare(b.name));
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
  private saveCurrent(): ProjectSummary {
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
        if (this.dirty) this.saveCurrent();
        const project = createEmptyProject(this.newId, name, c.id);
        this.storeProject(project, this.wallNow());
        this.loadProject(project);
        this.emitListChanged();
        return { type: "Project", project: this.project };
      }
      case "Open": {
        const entry = this.stored(c.id);
        if (this.dirty) this.saveCurrent();
        // Re-read after a possible autosave (opening the current project reloads it).
        this.loadProject(parseEtherFile(this.store.get(c.id)?.json ?? entry.json));
        return { type: "Project", project: this.project };
      }
      case "Save":
        return { type: "Saved", project: this.saveCurrent() };
      case "SaveAs": {
        const name = this.checkNewProject(c.new_id, c.name);
        const copy: Project = { ...wire(this.project), id: c.new_id };
        copy.settings = { ...copy.settings, name };
        this.storeProject(copy, this.wallNow());
        this.loadProject(copy);
        this.emitListChanged();
        return { type: "Project", project: this.project };
      }
      case "Duplicate": {
        const src = this.stored(c.id);
        const name = this.checkNewProject(c.new_id, c.name);
        const copy: Project = { ...parseEtherFile(src.json), id: c.new_id };
        copy.settings = { ...copy.settings, name };
        const summary = this.storeProject(copy, this.wallNow());
        this.emitListChanged();
        return { type: "Saved", project: summary };
      }
      case "Rename": {
        if (c.id === this.project.id) {
          return this.applyDocument([{ domain: "Project", command: c }], "Rename Project", gesture);
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
      case "Delete":
        if (c.id === this.project.id) fail("InvalidState", "cannot delete the open project");
        this.stored(c.id);
        this.store.delete(c.id);
        this.emitListChanged();
        return UNIT;
      case "SetScale":
        // A document edit (applied by `documentReducer`).
        return this.applyDocument([{ domain: "Project", command: c }], "Set Scale", gesture);
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
      case "Locate":
        this.position = Math.max(0, c.position);
        this.startPosition = this.position;
        break;
      case "TapTempo": {
        const now = this.now();
        this.tapTimes = [...this.tapTimes.filter((t) => now - t < 2000), now].slice(-4);
        if (this.tapTimes.length < 2) return { type: "Tempo", bpm: null };
        const first = this.tapTimes[0]!;
        const interval = (now - first) / (this.tapTimes.length - 1);
        const bpm = Math.round((60000 / interval) * 100) / 100;
        this.applyDocument([cmd("Transport", { type: "SetTempo", bpm })], "Tap Tempo", null);
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
    this.playing = true;
    this.position = this.startPosition;
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
    if (c.type === "ListInputs") {
      return {
        type: "Inputs",
        inputs: {
          audio: [
            { index: 0, name: "Mock In 1" },
            { index: 1, name: "Mock In 2" },
          ],
          midi: [{ id: "mock-midi", name: "Mock MIDI Keyboard" }],
        },
      };
    }
    if (c.type === "SetRecording") {
      const reply = this.liveRecord.set(c.enabled);
      this.syncTransport();
      return reply;
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
        return { type: "Locations", locations: [...MOCK_LOCATIONS] };
      case "ListDirectory":
        return { type: "Directory", listing: { location: c.location, path: normalize(c.path), entries: this.listDirectory(c.location, c.path) } };
      case "Import": {
        if (this.project.media[c.id]) fail("InvalidArgument", `media ${c.id} already exists`);
        const media = this.resolveImport(c.id, c.source);
        const value = this.transact("Import", gesture, (tx) => {
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
    }
  }

  private listDirectory(location: BrowseLocation, path: string) {
    if (location.type === "Library") {
      if (location.id !== LIBRARY_ID) fail("NotFound", `location ${location.id}`);
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
        return (
          Object.values(this.project.media).find((m) => m.file === `media/${normalize(source.path)}`) ??
          fail("NotFound", `file ${source.path}`)
        );
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
      // `media-references`: library files are referenced in place (like the desktop).
      location: { type: "External", path: libraryPath(path) },
    };
  }

  private engineCommand(c: EngineCommand): ReplyValue {
    const current = { backend: "mock", host: null, output_device: "Mock Output", input_device: "Mock Input", sample_rate: 48000, buffer_size: 256 };
    switch (c.type) {
      case "GetStatus":
        return {
          type: "Status",
          status: { running: true, backend: "mock", sample_rate: 48000, buffer_size: 256, output_latency: 256, input_latency: 256, xruns: 0, instance: "mock" },
        };
      case "ListAudioDevices":
        return {
          type: "AudioDevices",
          devices: {
            backends: ["mock"],
            hosts: [],
            outputs: [{ name: "Mock Output", channels: 2, sample_rates: [44100, 48000], is_default: true }],
            inputs: [{ name: "Mock Input", channels: 2, sample_rates: [44100, 48000], is_default: true }],
            current,
          },
        };
      case "SetAudioConfig":
        return fail("Unsupported", "the mock engine has no audio devices");
    }
  }

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
    this.exports.step();
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
      const level = t.mixer.mute ? 0 : Math.min(1.2, input) * dbToLinear(t.mixer.volume);
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
  const file: EtherFile = { format: ETHER_FORMAT, version: ETHER_VERSION, app_version: APP_VERSION, project };
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
  if (typeof file.version !== "number" || file.version > ETHER_VERSION) {
    return fail("Decode", `unsupported project version ${String(file.version)}`);
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
