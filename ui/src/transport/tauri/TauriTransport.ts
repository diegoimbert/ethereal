/**
 * TauriTransport — desktop host transport (Tauri v2 IPC to `ether-native`).
 *
 * Wire (see `apps/desktop/src-tauri/src/lib.rs`):
 * - `invoke("ether_connect", { messages, playhead, meters })` registers three Tauri
 *   `Channel`s. `messages` carries `ServerMessage`s of kind `Reply` / `Event` in the order
 *   the controller produced them (so a command's `Event::Patch`es always arrive before its
 *   `Reply`); `playhead` carries `PlayheadFrame`s (~60 Hz) and `meters` `MeterFrame`s
 *   (~30 Hz).
 * - `invoke("ether_send_batch", { messages })` queues `ClientMessage`s in order; each reply arrives on
 *   `messages`, matched by `id`.
 * - `invoke("ether_disconnect")` drops the channels (on `dispose`).
 * - `ethereal://` deep links: the shell emits `DEEP_LINK_EVENT` when one
 *   arrives and `invoke("take_deep_links")` drains them (`onDeepLink`).
 *
 * `connect()` resolves with the current project, or `null` when the engine has none open
 * (fresh start: nothing is opened on launch, the project screen picks one).
 *
 * Commands sent before `connect()` (the app's first queries run as it mounts, before the
 * provider connects) wait until the channels are registered: the host drops what it
 * answers while no UI is subscribed, which left those promises pending forever (e.g. the
 * device library's factory list, cached per transport, stayed empty for the session). A
 * reconnect keeps taking replies from the previous `messages` channel, for the commands in
 * flight when it swapped.
 */

import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { open as tauriOpen, save as tauriSave } from "@tauri-apps/plugin-dialog";
import type { ClientMessage, Command, Event, MeterFrame, PlayheadFrame, Project, ReplyValue, ServerMessage } from "@/generated";
import { t } from "@/i18n";
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
export type SaveDialogFn = (options: { title: string; defaultPath?: string; filters: { name: string; extensions: string[] }[] }) => Promise<string | null>;

/** Shell event carrying OS paths dropped on the window (`apps/desktop/src-tauri`). */
export const PATH_DROP_EVENT = "ether://path-drop";

/** Shell event: an `ethereal://` link arrived (no payload; drain with `take_deep_links`). */
export const DEEP_LINK_EVENT = "ether://deep-link";

/** Extensions offered by the import dialog (the engine's audio formats). */
export const AUDIO_DIALOG_EXTENSIONS = ["wav", "wave", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga", "m4a", "aac"] as const;

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

function notConnected(command: Command, cause: unknown): CommandFailedError {
  // Users read the key's text.
  return new CommandFailedError(
    { code: "InvalidState", message: `not connected: ${String(cause)}`, key: { id: "error.transport.engine_not_connected", args: {} } },
    command,
  ); // i18n-ignore: an internal error (logs, agents)
}

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
   * `connected`: `ether_connect` succeeded, the host has our channels and sends go out.
   * Before that (`idle`, `connecting`) they wait in `outbox`; after a failed connect they
   * fail right away until the next `connect()`.
   */
  private link: "idle" | "connecting" | "connected" | "failed" = "idle";
  /**
   * Tauri v2 doesn't guarantee that separate `invoke`s reach the backend in call order, so
   * one `ether_send_batch` is in flight at a time: it is accepted as soon as the host queued
   * its messages (not when they're handled). Commands sent meanwhile wait in `outbox` and go
   * together in the next invoke, so a slow IPC round trip (a busy main thread) costs one
   * round trip per burst, not one per command.
   */
  private outbox: Array<{ id: number; message: ClientMessage; command: Command }> = [];
  private sending = false;

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
    // The previous channels: only the replies still on their way (the new channel gets the rest).
    const [previous, ...frames] = this.channels;
    if (previous) previous.onmessage = (m) => (m as ServerMessage).kind === "Reply" && this.onServerMessage(m as ServerMessage);
    for (const c of frames) c.onmessage = noop;
    this.channels = [messages, playhead, meters] as ChannelLike<unknown>[];
    this.link = "connecting";
    try {
      this.info = (await this.invoke("ether_connect", { messages, playhead, meters })) as EngineInfo;
    } catch (e) {
      // What waited for the connection fails with it (callers may retry).
      this.link = "failed";
      for (const { id, command } of this.outbox.splice(0)) {
        const p = this.pending.get(id);
        this.pending.delete(id);
        p?.reject(notConnected(command, e));
      }
      throw e;
    }
    if (this.disposed) throw new Error("TauriTransport disposed");
    this.link = "connected";
    this.pump();
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
      return Promise.reject(new CommandFailedError({ code: "InvalidState", message: "transport disposed" }, command)); // i18n-ignore: an internal error (logs, agents)
    }
    if (this.link === "failed") return Promise.reject(notConnected(command, "the last connect failed"));
    const id = this.nextId++;
    const message: ClientMessage = { id, gesture: opts?.gesture ?? null, command };
    return new Promise<ReplyValue>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, command });
      this.outbox.push({ id, message, command });
      this.pump();
    });
  }

  /** Send everything queued in one invoke, unless one is in flight (it calls back here). */
  private pump(): void {
    if (this.sending || this.disposed || this.link !== "connected" || this.outbox.length === 0) return;
    const batch = this.outbox.splice(0);
    this.sending = true;
    // `new Promise`: a synchronous throw from `invoke` rejects too.
    new Promise((resolve) => resolve(this.invoke("ether_send_batch", { messages: batch.map((b) => b.message) })))
      .catch((e: unknown) => {
        for (const { id, command } of batch) {
          const p = this.pending.get(id);
          if (!p) continue;
          this.pending.delete(id);
          p.reject(new CommandFailedError({ code: "Internal", message: `ipc: ${String(e)}` }, command)); // i18n-ignore: an internal error (logs, agents)
        }
      })
      .finally(() => {
        this.sending = false;
        this.pump();
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

  // ─── `ethereal://` deep links ─────────────────────────

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
      title: t("files.dialog.import_audio"),
      filters: [{ name: t("files.filter.audio"), extensions: [...AUDIO_DIALOG_EXTENSIONS] }],
    });
    if (picked === null) return null;
    return (Array.isArray(picked) ? picked : [picked]).filter((p): p is string => typeof p === "string");
  }

  /** The OS folder dialog (the Relink dialog's folder search, `title` by default): an absolute path, or `null`. */
  async pickFolder(title?: string): Promise<string | null> {
    const picked = await this.openDialog({ multiple: false, directory: true, title: title ?? t("files.dialog.search_folder"), filters: [] });
    const path = Array.isArray(picked) ? picked[0] : picked;
    return typeof path === "string" ? path : null;
  }

  /**
   * The OS save dialog for a project bundle: an absolute path ending in `.ether`
   * (added when the dialog leaves it out), or `null` when dismissed. The engine writes it.
   */
  async pickBundleSavePath(defaultName: string): Promise<string | null> {
    const path = await this.saveDialog({
      title: t("files.dialog.export_project"),
      defaultPath: defaultName,
      filters: [{ name: t("files.filter.project"), extensions: ["ether"] }],
    });
    if (typeof path !== "string" || !path) return null;
    return /\.ether$/i.test(path) ? path : `${path}.ether`;
  }

  /** The OS save dialog for a MIDI file: an absolute path ending in `.mid`, or `null`. */
  async pickMidiSavePath(defaultName: string): Promise<string | null> {
    const path = await this.saveDialog({
      title: t("transport.dialog.export_midi"),
      defaultPath: defaultName,
      filters: [{ name: t("transport.filter.midi"), extensions: ["mid", "midi"] }],
    });
    if (typeof path !== "string" || !path) return null;
    return /\.midi?$/i.test(path) ? path : `${path}.mid`;
  }

  /** The OS save dialog for a project of another DAW (absolute path), or `null`. */
  async pickProjectExportPath(defaultName: string, extension: string, filterName: string): Promise<string | null> {
    const path = await this.saveDialog({
      title: t("files.dialog.export_daw"),
      defaultPath: defaultName,
      filters: [{ name: filterName, extensions: [extension] }],
    });
    if (typeof path !== "string" || !path) return null;
    return path.toLowerCase().endsWith(`.${extension}`) ? path : `${path}.${extension}`;
  }

  /** The OS open dialog for a project bundle (absolute path), or `null`. */
  async pickBundleFile(): Promise<string | null> {
    const picked = await this.openDialog({
      multiple: false,
      directory: false,
      title: t("files.dialog.import_project"),
      filters: [{ name: t("files.filter.project"), extensions: ["ether"] }],
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    return typeof path === "string" ? path : null;
  }

  /**
   * The OS open dialog for a project of another DAW (`Project::Import {
   * source: Path }`): an absolute path, or `null` when dismissed.
   */
  async pickImportFile(extensions: readonly string[] = ["dawproject", "als", "flp", "rpp", "rpp-bak"]): Promise<string | null> {
    const picked = await this.openDialog({
      multiple: false,
      directory: false,
      title: t("files.dialog.import_daw"),
      filters: [{ name: t("files.filter.daw_projects"), extensions: [...extensions] }],
    });
    const path = Array.isArray(picked) ? picked[0] : picked;
    return typeof path === "string" ? path : null;
  }

  /** The remembered collaboration token (app data dir, owner-only file). */
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
      p.reject(new CommandFailedError({ code: "InvalidState", message: "transport disposed" }, p.command)); // i18n-ignore: an internal error (logs, agents)
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
    throw new CommandFailedError({ code: "Internal", message: `expected a project reply, got ${reply.type}` }); // i18n-ignore: an internal error (logs, agents)
  }
  return reply.project;
}
