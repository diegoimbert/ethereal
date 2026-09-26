/**
 * WasmTransport — browser host transport. The engine runs as two wasm instances (see
 * `crates/ether-wasm`): the controller in a Web Worker and the engine in an AudioWorklet,
 * talking over SharedArrayBuffer rings. This class only speaks the UI side of it: JSON
 * `ClientMessage`s to the Worker, JSON `ServerMessage` batches back.
 *
 * The Worker/Worklet plumbing lives in the host app (`apps/web/src/engine`), which passes a
 * {@link WasmEndpoint}; the UI package stays free of wasm and worker URLs.
 *
 * OWNERSHIP: the `wasm-host` node owns this folder (`ui/src/transport/wasm/**`).
 *
 * Contract (`../EngineTransport.ts`): the Worker answers each message with one batch in
 * which the command's patches precede its reply, so listeners see the patches before the
 * `send()` promise settles.
 */

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

/** Connection to the controller Worker, provided by the host app. */
export interface WasmEndpoint {
  /** Boot the Worker + Worklet. Rejects when the browser can't run the engine. */
  start(): Promise<void>;
  /** Send one JSON-encoded `ClientMessage`. */
  post(messageJson: string): void;
  /** JSON arrays of `ServerMessage`s, in order. */
  onMessages(listener: (batchJson: string) => void): Unsubscribe;
  /** The engine died (worker/worklet error). Pending commands are rejected. */
  onFatal(listener: (error: Error) => void): Unsubscribe;
  dispose(): void;
}

export interface WasmTransportOptions {
  /** The engine endpoint (apps/web: `createWebEndpoint()`). Without one, `connect()` rejects. */
  endpoint?: WasmEndpoint;
  /** Name of the project created when the store is empty on first connect. */
  defaultProjectName?: string;
}

interface Pending {
  resolve: (value: ReplyValue) => void;
  reject: (error: unknown) => void;
  command: Command;
}

export class WasmTransport implements EngineTransport {
  /** Implemented; the web build constructs it with an endpoint (apps/web). */
  static readonly implemented: boolean = true;

  readonly kind = "wasm" as const;

  private readonly endpoint: WasmEndpoint | undefined;
  private readonly defaultProjectName: string;
  private readonly events = new Emitter<Event>();
  private readonly playhead = new Emitter<PlayheadFrame>();
  private readonly meters = new Emitter<MeterFrame>();
  private readonly pending = new Map<number, Pending>();
  private readonly subscriptions: Unsubscribe[] = [];
  private nextId = 1;
  private started: Promise<void> | null = null;
  private fatal: Error | null = null;
  private disposed = false;

  constructor(opts: WasmTransportOptions = {}) {
    this.endpoint = opts.endpoint;
    this.defaultProjectName = opts.defaultProjectName ?? "Untitled";
  }

  /**
   * Boot the engine and return the current project. If none is open yet (fresh Worker),
   * open the most recently saved one, or create a new one when the store is empty.
   */
  async connect(): Promise<Project> {
    await this.start();
    try {
      return this.projectOf(await this.send({ domain: "Project", command: { type: "Get" } }));
    } catch (e) {
      if (!(e instanceof CommandFailedError)) throw e;
    }
    const list = await this.send({ domain: "Project", command: { type: "List" } });
    const newest = list.type === "Projects" ? list.projects[0] : undefined;
    const reply = newest
      ? await this.send({ domain: "Project", command: { type: "Open", id: newest.id } })
      : await this.send({
          domain: "Project",
          command: { type: "Create", id: newProjectId(), name: this.defaultProjectName },
        });
    return this.projectOf(reply);
  }

  send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    if (this.disposed) return Promise.reject(new CommandFailedError({ code: "InvalidState", message: "transport disposed" }, command));
    if (this.fatal) return Promise.reject(new CommandFailedError({ code: "Internal", message: this.fatal.message }, command));
    const endpoint = this.endpoint;
    if (!endpoint) return Promise.reject(new Error("WasmTransport: no web engine endpoint configured"));
    const id = this.nextId++;
    const message: ClientMessage = { id, gesture: opts?.gesture ?? null, command };
    return new Promise<ReplyValue>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, command });
      // Commands wait for the Worker to boot (they queue in order behind the same promise).
      this.start().then(
        () => endpoint.post(JSON.stringify(message)),
        (e: unknown) => this.settleWithError(id, e),
      );
    });
  }

  onEvent(listener: (event: Event) => void): Unsubscribe {
    return this.events.on(listener);
  }

  subscribePlayhead(listener: (frame: PlayheadFrame) => void): Unsubscribe {
    return this.playhead.on(listener);
  }

  subscribeMeters(listener: (frame: MeterFrame) => void): Unsubscribe {
    return this.meters.on(listener);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.failPending(new Error("transport disposed"));
    for (const off of this.subscriptions.splice(0)) off();
    this.events.clear();
    this.playhead.clear();
    this.meters.clear();
    this.endpoint?.dispose();
  }

  // ─── internals ────────────────────────────────────────────────────────────────────────

  private start(): Promise<void> {
    if (this.disposed) return Promise.reject(new Error("WasmTransport disposed"));
    const endpoint = this.endpoint;
    if (!endpoint) return Promise.reject(new Error("WasmTransport: no web engine endpoint configured"));
    if (!this.started) {
      this.subscriptions.push(
        endpoint.onMessages((batch) => this.receive(batch)),
        endpoint.onFatal((error) => this.onFatal(error)),
      );
      this.started = endpoint.start();
    }
    return this.started;
  }

  private projectOf(reply: ReplyValue): Project {
    if (reply.type !== "Project") throw new Error(`WasmTransport: expected a Project reply, got ${reply.type}`);
    return reply.project;
  }

  private receive(batchJson: string): void {
    if (this.disposed) return;
    let batch: ServerMessage[];
    try {
      batch = JSON.parse(batchJson) as ServerMessage[];
    } catch (e) {
      console.error("WasmTransport: malformed message batch", e);
      return;
    }
    for (const message of batch) {
      switch (message.kind) {
        case "Reply": {
          const { id, result } = message.body;
          const p = this.pending.get(id);
          if (!p) break;
          this.pending.delete(id);
          if (result.status === "Ok") p.resolve(result.value);
          else p.reject(new CommandFailedError(result.error, p.command));
          break;
        }
        case "Event":
          this.events.emit(message.body);
          break;
        case "Playhead":
          this.playhead.emit(message.body);
          break;
        case "Meters":
          this.meters.emit(message.body);
          break;
      }
    }
  }

  private onFatal(error: Error): void {
    if (this.fatal) return;
    this.fatal = error;
    this.failPending(error);
    this.events.emit({ type: "Notification", level: "Error", message: `Audio engine stopped: ${error.message}` });
  }

  private settleWithError(id: number, error: unknown): void {
    const p = this.pending.get(id);
    if (!p) return;
    this.pending.delete(id);
    p.reject(error);
  }

  private failPending(error: Error): void {
    for (const [id, p] of [...this.pending]) {
      this.pending.delete(id);
      p.reject(new CommandFailedError({ code: "Internal", message: error.message }, p.command));
    }
  }
}
