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
 *
 * `connect()` resolves with the current project. If the engine has none open yet (fresh
 * start), it opens the most recently saved project from the engine-side store, or creates
 * an "Untitled" one when the store is empty.
 */

import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
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
import { newProjectId } from "../ids";

/** `invoke` from `@tauri-apps/api/core` (injectable for tests). */
export type InvokeFn = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** Minimal channel surface (a Tauri `Channel`; injectable for tests). */
export interface ChannelLike<T> {
  onmessage: (message: T) => void;
}

export interface TauriTransportOptions {
  invoke?: InvokeFn;
  createChannel?: <T>() => ChannelLike<T>;
  /** Name of the project created when the store is empty (default "Untitled"). */
  untitledName?: string;
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
  private readonly createChannel: <T>() => ChannelLike<T>;
  private readonly untitledName: string;
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
    this.createChannel = options.createChannel ?? (<T>() => new Channel<T>() as ChannelLike<T>);
    this.untitledName = options.untitledName ?? "Untitled";
  }

  async connect(): Promise<Project> {
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

  /** The open project; opens the newest stored project (or creates one) if none is open. */
  private async currentProject(): Promise<Project> {
    try {
      return projectOf(await this.send({ domain: "Project", command: { type: "Get" } }));
    } catch (e) {
      if (!(e instanceof CommandFailedError) || (e.code !== "InvalidState" && e.code !== "NotFound")) throw e;
    }
    const list = await this.send({ domain: "Project", command: { type: "List" } });
    const projects = list.type === "Projects" ? list.projects : [];
    const newest = [...projects].sort((a, b) => b.modified_ms - a.modified_ms)[0];
    const reply = newest
      ? await this.send({ domain: "Project", command: { type: "Open", id: newest.id } })
      : await this.send({ domain: "Project", command: { type: "Create", id: newProjectId(), name: this.untitledName } });
    return projectOf(reply);
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
