/**
 * TauriTransport — desktop host transport (Tauri v2 IPC to `ether-native`).
 *
 * OWNERSHIP: the `native-host` node owns this folder (`ui/src/transport/tauri/**`).
 *
 * Wire (see `apps/desktop/src-tauri/src/lib.rs`):
 * - `invoke("ether_connect", { messages, playhead, meters })` registers three Tauri
 *   `Channel`s. `messages` carries `ServerMessage`s of kind `Reply` / `Event` in the order
 *   the controller produced them (so a command's `Event::Patch`es always arrive before its
 *   `Reply`); `playhead` carries `PlayheadFrame`s (~60 Hz) and `meters` `MeterFrame`s
 *   (~30 Hz).
 * - `invoke("ether_send", { message })` queues one `ClientMessage`; its reply arrives on
 *   `messages`, matched by `id`.
 * - `invoke("ether_disconnect")` drops the channels (on `dispose`).
 * - `ethereal://` deep links (docs/SHARING.md §5): the shell emits `DEEP_LINK_EVENT` when one
 *   arrives and `invoke("take_deep_links")` drains them (`onDeepLink`).
 *
 * `connect()` resolves with the current project, or `null` when the engine has none open
 * (fresh start: base-131, nothing is opened on launch, the project screen picks one).
 */

import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { open as tauriOpen, save as tauriSave } from "@tauri-apps/plugin-dialog";
import type {
  ClientMessage,
  Command,
  Event,
  MeterFrame,
  PlayheadFrame,
  Project,
  ReplyValue,
  ServerMessage,
} from "@/generated";
import { CommandFailedError, Emitter, type EngineTransport, type SendOptions, type Unsubscribe } from "../EngineTransport";

/** `invoke` from `@tauri-apps/api/core` (injectable for tests). */
export type InvokeFn = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** Minimal channel surface (a Tauri `Channel`; injectable for tests). */
export interface ChannelLike<T> {
  onmessage: (message: T) => void;
}

/** `listen` from `@tauri-apps/api/event` (injectable for tests). */
export type ListenFn = <T>(event: string, handler: (e: { payload: T }) => void) => Promise<() => void>;

/** `open` from `@tauri-apps/plugin-dialog` (injectable for tests). */
export type OpenDialogFn = (options: {
  multiple: boolean;
  directory: boolean;
  title: string;
  filters: { name: string; extensions: string[] }[];
}) => Promise<string | string[] | null>;

/** `save` from `@tauri-apps/plugin-dialog` (injectable for tests). */
export type SaveDialogFn = (options: {
  title: string;
  defaultPath?: string;
  filters: { name: string; extensions: string[] }[];
}) => Promise<string | null>;

/** Shell event carrying OS paths dropped on the window (`apps/desktop/src-tauri`). */
export const PATH_DROP_EVENT = "ether://path-drop";

/** Shell event: an `ethereal://` link arrived (no payload; drain with `take_deep_links`). */
export const DEEP_LINK_EVENT = "ether://deep-link";

/** Extensions offered by the import dialog (the engine's audio formats). */
export const AUDIO_DIALOG_EXTENSIONS = ["wav", "wave", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga"] as const;

export interface TauriTransportOptions {
  invoke?: InvokeFn;
  listen?: ListenFn;
  openDialog?: OpenDialogFn;
  saveDialog?: SaveDialogFn;
  createChannel?: <T>() => ChannelLike<T>;
}

/** Info returned by `ether_connect` (diagnostics). */
export interface EngineInfo {
  version: string;
  instance: string;
  data_dir: string;
  projects_root: string;
  backend: string;
  sample_rate: number;
  buffer_size: number;
  device: string | null;
}

interface Pending {
  resolve: (value: ReplyValue) => void;
  reject: (error: unknown) => void;
  command: Command;
}

const noop = () => {};

export class TauriTransport implements EngineTransport {
  /** `createDefaultTransport` uses this transport inside Tauri once implemented. */
  static readonly implemented: boolean = true;

  readonly kind = "tauri" as const;

  /** Engine diagnostics from the last `connect()`. */
  info: EngineInfo | null = null;

  private readonly invoke: InvokeFn;
  private readonly listen: ListenFn;
  private readonly openDialog: OpenDialogFn;
  private readonly saveDialog: SaveDialogFn;
  private readonly createChannel: <T>() => ChannelLike<T>;
  private readonly events = new Emitter<Event>();
  private readonly playheadEmitter = new Emitter<PlayheadFrame>();
  private readonly meterEmitter = new Emitter<MeterFrame>();
  private readonly pending = new Map<number, Pending>();
  private channels: ChannelLike<unknown>[] = [];
  private nextId = 1;
  private disposed = false;
  /**
   * Tauri v2 doesn't guarantee that separate `invoke`s reach the backend in call order, so
   * `ether_send` calls are chained: each is issued only after the previous one was
   * accepted (which happens as soon as the host queued the message, not when it's handled).
   */
  private sendQueue: Promise<unknown> = Promise.resolve();

  constructor(options: TauriTransportOptions = {}) {
    this.invoke = options.invoke ?? ((cmd, args) => tauriInvoke(cmd, args));
    this.listen = options.listen ?? ((event, handler) => tauriListen(event, handler));
    this.openDialog = options.openDialog ?? ((o) => tauriOpen(o));
    this.saveDialog = options.saveDialog ?? ((o) => tauriSave(o));
    this.createChannel = options.createChannel ?? (<T>() => new Channel<T>() as ChannelLike<T>);
  }

  async connect(): Promise<Project | null> {
    if (this.disposed) throw new Error("TauriTransport disposed");
    const messages = this.createChannel<ServerMessage>();
    const playhead = this.createChannel<PlayheadFrame>();
    const meters = this.createChannel<MeterFrame>();
    messages.onmessage = (m) => this.onServerMessage(m);
    playhead.onmessage = (f) => this.playheadEmitter.emit(f);
    meters.onmessage = (f) => this.meterEmitter.emit(f);
    for (const c of this.channels) c.onmessage = noop;
    this.channels = [messages, playhead, meters] as ChannelLike<unknown>[];
    this.info = (await this.invoke("ether_connect", { messages, playhead, meters })) as EngineInfo;
    return this.currentProject();
  }

  /** The open project, or `null` when none is open (nothing is opened here). */
  private async currentProject(): Promise<Project | null> {
    try {
      return projectOf(await this.send({ domain: "Project", command: { type: "Get" } }));
    } catch (e) {
      if (e instanceof CommandFailedError && (e.code === "InvalidState" || e.code === "NotFound")) return null;
      throw e;
    }
  }

  send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    if (this.disposed) {
      return Promise.reject(new CommandFailedError({ code: "InvalidState", message: "transport disposed" }, command));
    }
    const id = this.nextId++;
    const message: ClientMessage = { id, gesture: opts?.gesture ?? null, command };
    return new Promise<ReplyValue>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, command });
      // Chained: issued only once the previous `ether_send` was accepted.
      const issued = this.sendQueue.then(() => (this.disposed ? undefined : this.invoke("ether_send", { message })));
      this.sendQueue = issued.catch(noop);
      issued.catch((e: unknown) => {
        if (!this.pending.delete(id)) return;
        reject(new CommandFailedError({ code: "Internal", message: `ipc: ${String(e)}` }, command));
      });
    });
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

  // ─── `file-import`: OS files by path (CONTRACTS.md §12.13) ───────────────────────────

  /**
   * OS paths dropped on the window (`PATH_DROP_EVENT`, emitted by the shell on macOS; other
   * platforms deliver drops to the page as files).
   */
  onPathDrop(listener: (paths: string[]) => void): Unsubscribe {
    let off: (() => void) | null = null;
    let cancelled = false;
    this.listen<{ paths: string[] }>(PATH_DROP_EVENT, (e) => {
      if (Array.isArray(e.payload?.paths)) listener(e.payload.paths.filter((p) => typeof p === "string"));
    })
      .then((unlisten) => {
        if (cancelled) unlisten();
        else off = unlisten;
      })
      .catch((e: unknown) => console.warn("[ethereal] path drops unavailable:", e));
    return () => {
      cancelled = true;
      off?.();
    };
  }

  // ─── `join-flow`: `ethereal://` deep links (docs/SHARING.md §5) ─────────────────────────

  /**
   * `ethereal://` links opened with the app: the one it was started with (cold start) and
   * every later one (warm start; a second launch hands its link to this instance). Each
   * link is delivered once, oldest first.
   */
  onDeepLink(listener: (url: string) => void): Unsubscribe {
    let off: (() => void) | null = null;
    let cancelled = false;
    let draining = Promise.resolve();
    const drain = () => {
      draining = draining
        .then(() => (cancelled ? [] : this.invoke("take_deep_links")))
        .then((urls) => {
          if (cancelled || !Array.isArray(urls)) return;
          for (const u of urls) if (typeof u === "string") listener(u);
        })
        .catch((e: unknown) => console.warn("[ethereal] deep links unavailable:", e));
    };
    this.listen<unknown>(DEEP_LINK_EVENT, drain)
      .then((unlisten) => {
        if (cancelled) unlisten();
        else off = unlisten;
      })
      .catch((e: unknown) => console.warn("[ethereal] deep links unavailable:", e))
      // Links that arrived before we listened (cold start).
      .finally(drain);
    return () => {
      cancelled = true;
      off?.();
    };
  }

  /** The OS file dialog for audio files: absolute paths, or `null` when dismissed. */
  async pickAudioFiles(): Promise<string[] | null> {
    const picked = await this.openDialog({
      multiple: true,
      directory: false,
      title: "Import audio",
      filters: [{ name: "Audio", extensions: [...AUDIO_DIALOG_EXTENSIONS] }],
    });
    if (picked === null) return null;
    return (Array.isArray(picked) ? picked : [picked]).filter((p): p is string => typeof p === "string");
  }

  /** The OS folder dialog (the Relink dialog's folder search): an absolute path, or `null`. */
  async pickFolder(): Promise<string | null> {
    const picked = await this.openDialog({ multiple: false, directory: true, title: "Search in folder", filters: [] });
    const path = Array.isArray(picked) ? picked[0] : picked;
    return typeof path === "string" ? path : null;
  }

  /**
   * base-114: the OS save dialog for a project bundle: an absolute path ending in `.ether`
   * (added when the dialog leaves it out), or `null` when dismissed. The engine writes it.
   */
  async pickBundleSavePath(defaultName: string): Promise<string | null> {
    const path = await this.saveDialog({
      title: "Export project",
      defaultPath: defaultName,
      filters: [{ name: "Ethereal project", extensions: ["ether"] }],
    });
    if (typeof path !== "string" || !path) return null;
    return /\.ether$/i.test(path) ? path : `${path}.ether`;
  }

  /** base-114: the OS open dialog for a project bundle (absolute path), or `null`. */
  async pickBundleFile(): Promise<string | null> {
    const picked = await this.openDialog({
      multiple: false,
      directory: false,
      title: "Import project",
      filters: [{ name: "Ethereal project", extensions: ["ether"] }],
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    return typeof path === "string" ? path : null;
  }

  /** base-114: the remembered collaboration token (app data dir, owner-only file). */
  async loadCollabToken(): Promise<string | null> {
    const token = await this.invoke("collab_token_load");
    return typeof token === "string" && token ? token : null;
  }

  /** Remember (or forget, with `null`) the collaboration token. */
  async saveCollabToken(token: string | null): Promise<void> {
    await this.invoke("collab_token_save", { token });
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const c of this.channels) c.onmessage = noop;
    this.channels = [];
    for (const [, p] of this.pending) {
      p.reject(new CommandFailedError({ code: "InvalidState", message: "transport disposed" }, p.command));
    }
    this.pending.clear();
    this.events.clear();
    this.playheadEmitter.clear();
    this.meterEmitter.clear();
    this.invoke("ether_disconnect").catch(noop);
  }

  private onServerMessage(m: ServerMessage): void {
    switch (m.kind) {
      case "Reply": {
        const p = this.pending.get(m.body.id);
        if (!p) return;
        this.pending.delete(m.body.id);
        const r = m.body.result;
        if (r.status === "Ok") p.resolve(r.value);
        else p.reject(new CommandFailedError(r.error, p.command));
        return;
      }
      case "Event":
        this.events.emit(m.body);
        return;
      case "Playhead":
        this.playheadEmitter.emit(m.body);
        return;
      case "Meters":
        this.meterEmitter.emit(m.body);
        return;
    }
  }
}

function projectOf(reply: ReplyValue): Project {
  if (reply.type !== "Project") {
    throw new CommandFailedError({ code: "Internal", message: `expected a Project reply, got ${reply.type}` });
  }
  return reply.project;
}
