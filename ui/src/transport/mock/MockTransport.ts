/**
 * MockTransport: an in-memory engine implementing `EngineTransport` with no Rust, so UI
 * nodes can build real features (and tests) against it. It is also the executable
 * reference of the transport contract; real hosts must behave the same way.
 *
 * ## Semantics (shared with the real engine)
 * - **Document**: a normalized `Project` (flat tables keyed by ULID, sibling order via
 *   fractional `OrderKey`s). `connect()` returns a copy; afterwards the UI only learns about
 *   changes through events.
 * - **Patches before replies**: a document command applies atomically, then emits ONE
 *   `Event::Patch` (whole-entity `Upsert`/`Remove`/`Settings` changes, `revision` + 1,
 *   current `HistoryState`), and only then settles the `send()` promise. After
 *   `await send(...)` the store mirror already reflects the command.
 * - **Errors**: failing commands change nothing and reject with `CommandFailedError`.
 * - **Client-side ids**: commands that create entities carry their ids (`newId()`); the
 *   engine only generates ids for entities it creates on its own (children of duplicated
 *   tracks/clips/scenes, the right part of an overlap split).
 * - **Undo**: every document command is one undo step. Commands sent with the same
 *   `gesture` merge into one step until `Edit::EndGesture` (or a command with another/no
 *   gesture). `Edit::Batch` applies several document commands as one all-or-nothing step
 *   (no transport commands, queries or nested batches inside). Undo/Redo emit patches.
 * - **Non-document state**: transport play state → `Event::Transport` (emitted on connect
 *   and whenever a field changes); session clip play states → `Event::Session`
 *   (`Queued` → `Playing` at the next quantization boundary, `Stopping` → `Stopped`);
 *   playhead (~60 Hz while playing) and meters (~30 Hz) on their own streams.
 * - Everything crossing the "wire" is JSON-cloned, like a real host would serialize it.
 *
 * ## Mock limitations
 * - Tempo map is step-only (linear tempo ramps are treated as steps); scene tempo/time
 *   signature are stored but not applied on launch; Gate/Toggle/Repeat launch modes and
 *   legato behave like Trigger; `ReleaseClip`/`BackToArrangement` are no-ops.
 * - No audio. Meters are synthesized from what "would" play (clips under the playhead,
 *   playing session clips, volume/pan/mute); CPU load is fake.
 * - Replies `Err { code: "Unsupported" }`: plugins (insert/editor/sandbox/reload), file
 *   paths (`Project::Open/Save` with `Path`, `Save` without target, `Media::Import`,
 *   `Media::ListDirectory`), `Recording::SetRecording`, `Warp::DetectTempo`,
 *   `Engine::SetAudioConfig`.
 * - Harmless answers: `Plugin::List` → no plugins, `Plugin::Rescan` → an empty scan,
 *   `Media::Preview/StopPreview` → Unit, `Recording::ListInputs` / `Engine::*` → fake
 *   devices, `Media::GetPeaks` → a deterministic synthetic waveform.
 * - Validation covers the common invariants, not all of them (e.g. routing cycles through
 *   group outputs are not detected).
 *
 * ## Testing
 * `new MockTransport({ timers: "manual" })` disables real timers; drive time with
 * `tick(ms)`. `seed` makes ids and meter noise deterministic.
 */

import type {
  ClipId,
  ClipStateChange,
  Command,
  EditCommand,
  EngineCommand,
  EtherFile,
  Event,
  GestureId,
  HistoryState,
  MediaCommand,
  MeterFrame,
  PatchChange,
  PlayheadFrame,
  PluginCommand,
  Project,
  ProjectCommand,
  Quantization,
  RecordingCommand,
  ReplyValue,
  SessionCommand,
  SessionPlayback,
  TrackId,
  TrackMeter,
  TransportCommand,
  TransportState,
} from "@/generated";
import { cmd } from "../cmd";
import { CommandFailedError, Emitter, type EngineTransport, type SendOptions, type Unsubscribe } from "../EngineTransport";
import { newId as defaultNewId } from "../ids";
import { createDemoProject, createEmptyProject } from "./demoProject";
import { fail, isDocumentCommand, labelOf, reduceDocumentCommand, sessionClipsInScene } from "./documentReducer";
import { synthesizePeaks } from "./peaks";
import { mulberry32, seededIdFactory } from "./random";
import { beatsPerBar, beatsToSeconds, bpmAt, nextGridLine, signatureAt } from "./tempo";
import { changeKey, Tx } from "./tx";

export interface MockTransportOptions {
  /** Initial document (default: `createDemoProject()`). Copied, never mutated. */
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
export const ETHER_VERSION = 1;
const APP_VERSION = "0.0.1-mock";

const PLAYHEAD_INTERVAL_MS = 16;
const METER_INTERVAL_MS = 33;
const EPS = 1e-9;
const UNIT: ReplyValue = { type: "Unit" };

interface HistoryEntry {
  label: string;
  gesture: GestureId | null;
  /** Final entity states (reapply to redo). */
  redo: PatchChange[];
  /** Initial entity states (reapply to undo). */
  undo: PatchChange[];
}

/** Per-track session playback runtime. Times are in `elapsed` beats (monotonic). */
interface SlotRuntime {
  playing: { clip: ClipId; since: number } | null;
  queued: { clip: ClipId; at: number } | null;
  stopAt: number | null;
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
  /** Beats elapsed while playing (never wraps; used for session quantization). */
  private elapsed = 0;
  private playheadDirty = true;
  private lastTransportJson = "";
  private tapTimes: number[] = [];

  // Session + meters runtime.
  private readonly slots = new Map<TrackId, SlotRuntime>();
  private readonly levels = new Map<TrackId, number>();
  private metersSilent = false;

  constructor(opts: MockTransportOptions = {}) {
    this.project = wire(opts.project ?? createDemoProject());
    this.latencyMs = Math.max(0, opts.latencyMs ?? 0);
    this.manual = opts.timers === "manual";
    this.historyLimit = opts.historyLimit ?? 500;
    this.newId = opts.seed !== undefined ? seededIdFactory(opts.seed + 1000) : defaultNewId;
    this.rand = mulberry32(opts.seed ?? 1);
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
   * playhead, fires session boundaries and emits playhead/meter frames.
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

  // ─── Dispatch ─────────────────────────────────────────────────────────────────────────

  private execute(command: Command, gesture: GestureId | null): ReplyValue {
    const isEndGesture = command.domain === "Edit" && command.command.type === "EndGesture";
    if (!isEndGesture && gesture !== this.openGesture) this.openGesture = null;

    if (isDocumentCommand(command)) return this.applyDocument([command], labelOf(command), gesture);

    switch (command.domain) {
      case "Transport":
        return this.transportCommand(command.command);
      case "Project":
        return this.projectCommand(command.command);
      case "Edit":
        return this.editCommand(command.command, gesture);
      case "Session":
        return this.sessionCommand(command.command);
      case "Device":
        // Queries (ListBuiltin / GetDescriptor): run the reducer on a throwaway transaction.
        return reduceDocumentCommand({ tx: new Tx(this.project), newId: this.newId, position: this.position }, command);
      case "Plugin":
        return this.pluginCommand(command.command);
      case "Recording":
        return this.recordingCommand(command.command);
      case "Media":
        return this.mediaCommand(command.command);
      case "Engine":
        return this.engineCommand(command.command);
      case "Warp":
        return fail("Unsupported", "tempo detection is not available in the mock engine");
      default:
        return fail("InvalidArgument", `unknown command domain`);
    }
  }

  /** Apply document commands as one transaction / undo step and emit its patch. */
  private applyDocument(commands: Command[], label: string, gesture: GestureId | null): ReplyValue {
    const tx = new Tx(this.project);
    const ctx = { tx, newId: this.newId, position: this.position };
    let value: ReplyValue = UNIT;
    try {
      for (const c of commands) value = reduceDocumentCommand(ctx, c);
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
    // Session runtime must forget removed clips (the UI store drops their states itself).
    for (const c of changes) {
      if (c.type !== "Remove" || c.key.type !== "Clip") continue;
      for (const rt of this.slots.values()) {
        if (rt.playing?.clip === c.key.id) rt.playing = null;
        if (rt.queued?.clip === c.key.id) rt.queued = null;
      }
    }
  }

  private transportState(): TransportState {
    const s = this.project.settings;
    return {
      playing: this.playing,
      recording: false,
      loop_enabled: s.loop_enabled,
      loop_region: s.loop_region,
      bpm: bpmAt(this.project, this.position),
      time_signature: signatureAt(this.project, this.position),
      metronome: s.metronome,
      launch_quantization: s.launch_quantization,
      start_position: this.startPosition,
    };
  }

  /** Emit `Event::Transport` if any field changed (or `force`). */
  private syncTransport(force = false): void {
    const state = this.transportState();
    const json = JSON.stringify(state);
    if (!force && json === this.lastTransportJson) return;
    this.lastTransportJson = json;
    this.emit({ type: "Transport", state });
  }

  private loadProject(project: Project, path: string | null): void {
    this.project = project;
    this.undoStack = [];
    this.redoStack = [];
    this.openGesture = null;
    this.playing = false;
    this.position = 0;
    this.startPosition = 0;
    this.slots.clear();
    this.levels.clear();
    this.playheadDirty = true;
    this.emit({ type: "ProjectLoaded", project, path });
    this.syncTransport();
  }

  // ─── Project ──────────────────────────────────────────────────────────────────────────

  private projectCommand(c: ProjectCommand): ReplyValue {
    switch (c.type) {
      case "New":
        this.loadProject(createEmptyProject(this.newId), null);
        return { type: "Project", project: this.project };
      case "Get":
        return { type: "Project", project: this.project };
      case "Open": {
        if (c.source.type === "Path") return fail("Unsupported", "the mock engine cannot read files; use a Json source");
        this.loadProject(parseEtherFile(c.source.json), null);
        return { type: "Project", project: this.project };
      }
      case "Save": {
        if (c.target === null) return fail("InvalidState", "project was never saved; pass a target");
        if (c.target.type === "Path") return fail("Unsupported", "the mock engine cannot write files; use a Json target");
        const file: EtherFile = { format: ETHER_FORMAT, version: ETHER_VERSION, app_version: APP_VERSION, project: this.project };
        return { type: "Saved", path: null, json: JSON.stringify(file, null, 2) };
      }
      case "SetName":
        return fail("Internal", "unreachable: SetName is a document command");
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
    this.playing = false;
    const changes: ClipStateChange[] = [];
    for (const [track, rt] of this.slots) {
      if (rt.playing) changes.push({ track, clip: rt.playing.clip, state: "Stopped" });
      if (rt.queued && rt.queued.clip !== rt.playing?.clip) changes.push({ track, clip: rt.queued.clip, state: "Stopped" });
    }
    this.slots.clear();
    if (changes.length) this.emit({ type: "Session", changes });
  }

  // ─── Session ──────────────────────────────────────────────────────────────────────────

  private slot(track: TrackId): SlotRuntime {
    let rt = this.slots.get(track);
    if (!rt) this.slots.set(track, (rt = { playing: null, queued: null, stopAt: null }));
    return rt;
  }

  /** Quantization grid in beats at the playhead (0 = immediate). */
  private grid(q: Quantization): number {
    switch (q.type) {
      case "None":
        return 0;
      case "Bars":
        return q.count * beatsPerBar(signatureAt(this.project, this.position));
      case "Beats":
        return q.beats;
    }
  }

  /** `elapsed` time of the next boundary of `grid` (now if on one, or if grid is 0). */
  private boundary(grid: number): number {
    if (grid <= 0) return this.elapsed;
    return this.elapsed + (nextGridLine(this.position, grid, true) - this.position);
  }

  private launchClip(clipId: ClipId, immediate: boolean, out: ClipStateChange[]): void {
    const clip = this.project.clips[clipId] ?? fail("NotFound", `clip ${clipId}`);
    if (clip.location.type !== "Session") fail("InvalidArgument", "only session clips can be launched");
    const rt = this.slot(clip.track);
    if (rt.queued && rt.queued.clip !== clipId && rt.queued.clip !== rt.playing?.clip) {
      out.push({ track: clip.track, clip: rt.queued.clip, state: "Stopped" });
    }
    rt.queued = null;
    rt.stopAt = null;
    const at = immediate ? this.elapsed : this.boundary(this.grid(clip.launch.quantization ?? this.project.settings.launch_quantization));
    if (at <= this.elapsed + EPS) {
      if (rt.playing && rt.playing.clip !== clipId) out.push({ track: clip.track, clip: rt.playing.clip, state: "Stopped" });
      rt.playing = { clip: clipId, since: this.elapsed };
      out.push({ track: clip.track, clip: clipId, state: "Playing" });
    } else {
      rt.queued = { clip: clipId, at };
      out.push({ track: clip.track, clip: clipId, state: "Queued" });
    }
  }

  private stopTrack(track: TrackId, out: ClipStateChange[]): void {
    const rt = this.slots.get(track);
    if (!rt) return;
    if (rt.queued && rt.queued.clip !== rt.playing?.clip) out.push({ track, clip: rt.queued.clip, state: "Stopped" });
    rt.queued = null;
    if (!rt.playing) return;
    const at = this.playing ? this.boundary(this.grid(this.project.settings.launch_quantization)) : this.elapsed;
    if (at <= this.elapsed + EPS) {
      out.push({ track, clip: rt.playing.clip, state: "Stopped" });
      rt.playing = null;
      rt.stopAt = null;
    } else if (rt.stopAt === null) {
      rt.stopAt = at;
      out.push({ track, clip: rt.playing.clip, state: "Stopping" });
    }
  }

  private sessionCommand(c: SessionCommand): ReplyValue {
    const out: ClipStateChange[] = [];
    const wasStopped = !this.playing;
    switch (c.type) {
      case "LaunchClip":
        // Validate before starting the transport.
        if (!this.project.clips[c.clip]) fail("NotFound", `clip ${c.clip}`);
        if (this.project.clips[c.clip]!.location.type !== "Session") fail("InvalidArgument", "only session clips can be launched");
        if (wasStopped) this.startPlaying();
        this.launchClip(c.clip, wasStopped, out);
        break;
      case "LaunchScene": {
        if (!this.project.scenes[c.scene]) fail("NotFound", `scene ${c.scene}`);
        if (wasStopped) this.startPlaying();
        const clips = new Map(sessionClipsInScene(this.project, c.scene).map((cl) => [cl.track, cl.id]));
        for (const t of Object.values(this.project.tracks)) {
          if (t.kind !== "Audio" && t.kind !== "Midi") continue;
          const clip = clips.get(t.id);
          if (clip) this.launchClip(clip, wasStopped, out);
          else this.stopTrack(t.id, out);
        }
        break;
      }
      case "StopTrack":
        if (!this.project.tracks[c.track]) fail("NotFound", `track ${c.track}`);
        this.stopTrack(c.track, out);
        break;
      case "StopAll":
        for (const track of [...this.slots.keys()]) this.stopTrack(track, out);
        break;
      case "ReleaseClip":
      case "BackToArrangement":
        break; // Gate mode / arrangement override are not simulated.
      default:
        return fail("Internal", `unreachable: ${c.type} is a document command`);
    }
    if (out.length) this.emit({ type: "Session", changes: out });
    if (wasStopped && this.playing) {
      this.playheadDirty = true;
      this.syncTransport();
    }
    return UNIT;
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
    return fail("Unsupported", "recording is not available in the mock engine");
  }

  private mediaCommand(c: MediaCommand): ReplyValue {
    switch (c.type) {
      case "GetPeaks": {
        const media = this.project.media[c.request.media] ?? fail("NotFound", `media ${c.request.media}`);
        return { type: "Peaks", peaks: synthesizePeaks(media, c.request) };
      }
      case "Preview":
      case "StopPreview":
        return UNIT;
      default:
        return fail("Unsupported", "the mock engine has no file system");
    }
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

  // ─── Time ─────────────────────────────────────────────────────────────────────────────

  private now(): number {
    return this.manual ? this.manualNow : performance.now();
  }

  /** Advance the playhead to `now()`, fire session boundaries, publish a playhead frame. */
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
      this.elapsed += beats;
      this.advanceSession();
      this.syncTransport(); // tempo/signature at the playhead may change
    }
    if (this.playing || this.playheadDirty) this.emitPlayhead();
  }

  private advanceSession(): void {
    const out: ClipStateChange[] = [];
    for (const [track, rt] of this.slots) {
      if (rt.queued && this.elapsed >= rt.queued.at - EPS) {
        if (rt.playing && rt.playing.clip !== rt.queued.clip) out.push({ track, clip: rt.playing.clip, state: "Stopped" });
        rt.playing = { clip: rt.queued.clip, since: rt.queued.at };
        out.push({ track, clip: rt.queued.clip, state: "Playing" });
        rt.queued = null;
      }
      if (rt.playing && rt.stopAt !== null && this.elapsed >= rt.stopAt - EPS) {
        out.push({ track, clip: rt.playing.clip, state: "Stopped" });
        rt.playing = null;
        rt.stopAt = null;
      }
      if (rt.playing) {
        const clip = this.project.clips[rt.playing.clip];
        if (clip && !clip.looping.enabled && this.elapsed - rt.playing.since >= clip.length) {
          out.push({ track, clip: clip.id, state: "Stopped" });
          rt.playing = null;
          rt.stopAt = null;
        }
      }
    }
    if (out.length) this.emit({ type: "Session", changes: out });
  }

  private emitPlayhead(): void {
    this.playheadDirty = false;
    const session: SessionPlayback[] = [];
    for (const [track, rt] of this.slots) {
      if (!rt.playing) continue;
      const clip = this.project.clips[rt.playing.clip];
      if (!clip) continue;
      let pos = clip.offset + (this.elapsed - rt.playing.since);
      const { enabled, start, end } = clip.looping;
      if (enabled && end > start && pos >= end) pos = start + ((pos - end) % (end - start));
      session.push({ track, clip: clip.id, position: pos });
    }
    this.playheadEmitter.emit({
      transport: {
        position: this.position,
        seconds: beatsToSeconds(this.project, this.position),
        playing: this.playing,
        bpm: bpmAt(this.project, this.position),
      },
      session,
    });
  }

  /** How much signal a track "would" produce right now, 0..1 before its fader. */
  private activity(track: TrackId): number {
    if (!this.playing) return 0;
    if (this.slots.get(track)?.playing) return 0.6;
    for (const c of Object.values(this.project.clips)) {
      if (c.track !== track || c.muted || c.location.type !== "Arrangement") continue;
      if (this.position >= c.location.start && this.position < c.location.start + c.length) return 0.55;
    }
    return 0;
  }

  /** Synthesize one meter frame (peak-hold with decay, beat pulse, a bit of noise). */
  private meterStep(): void {
    const tracks = Object.values(this.project.tracks);
    const pulse = 0.7 + 0.3 * (1 - (this.position % 1));
    const decay = 0.82;
    const raw = new Map<TrackId, number>();
    for (const t of tracks) {
      if (t.kind === "Master" || t.kind === "Return" || t.kind === "Group") continue;
      raw.set(t.id, this.activity(t.id) * pulse * (0.9 + 0.2 * this.rand()));
    }
    for (const s of Object.values(this.project.sends)) {
      raw.set(s.to, (raw.get(s.to) ?? 0) + (raw.get(s.from) ?? 0) * dbToLinear(s.level) * 0.8);
    }
    for (const t of tracks) {
      if (t.kind === "Group") {
        let sum = 0;
        for (const k of tracks) if (k.parent === t.id) sum += raw.get(k.id) ?? 0;
        raw.set(t.id, sum * 0.7);
      }
    }
    const post = (id: TrackId) => {
      const t = this.project.tracks[id];
      return t && !t.mixer.mute ? (raw.get(id) ?? 0) * dbToLinear(t.mixer.volume) : 0;
    };
    let masterIn = 0;
    for (const t of tracks) if (t.kind !== "Master" && t.parent === null && t.output.type === "Master") masterIn += post(t.id);
    const master = tracks.find((t) => t.kind === "Master");
    if (master) raw.set(master.id, Math.min(1.2, masterIn * 0.6));

    const meters: TrackMeter[] = [];
    let silent = true;
    for (const t of tracks) {
      const level = Math.max(post(t.id), (this.levels.get(t.id) ?? 0) * decay);
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

/** Parse and minimally validate an `.ether` JSON document. */
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
  const tables = ["tracks", "clips", "notes", "devices", "sends", "scenes", "automation_lanes", "automation_points", "tempo_points", "time_signatures", "warp_markers", "media"] as const;
  if (!p || typeof p !== "object" || !p.settings || tables.some((t) => typeof p[t] !== "object" || p[t] === null)) {
    return fail("Decode", "malformed project");
  }
  if (Object.values(p.tracks).filter((t) => t.kind === "Master").length !== 1) return fail("Decode", "project must have exactly one master track");
  return p;
}
