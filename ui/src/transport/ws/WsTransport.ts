/**
 * WsTransport: the UI talking to a remote engine (`ether-server` or a desktop app) over a
 * WebSocket. Owned by the `remote-engine` node (see `crates/ether-protocol/src/remote.rs`
 * and docs/ROADMAP.md).
 *
 * Protocol:
 * 1. Open `ws(s)://host:port/` (never put the token in the URL).
 * 2. Send one text frame: `ClientHello { protocol_version, token, client }`.
 * 3. Receive one text frame: `ServerHello` — `Welcome { server, session }` or
 *    `Rejected { reason, message }` (then the server closes: 4001 auth, 4002 version,
 *    4003 busy). `open()` does steps 1-3 (the connect dialog uses it to report errors
 *    before switching transports).
 * 4. Afterwards every text frame is one JSON `ClientMessage` (sent) / `ServerMessage`
 *    (received), with the usual ordering (patches before the reply); `connect()` then gets
 *    the open project (opening the newest stored one, or creating one, when none is open;
 *    same as the desktop transport).
 * 5. Binary frames carry bulk bytes (`./binaryFrame.ts`): `[kind u8][header_len u32 LE]
 *    [header JSON][payload]`, kinds `Bytes` = 1 (upload chunks, export downloads) and
 *    `Peaks` = 2. Received binary frames are turned back into the plain JSON shapes, so
 *    features never see the difference. `uploadChunk()` sends raw bytes.
 *
 * Several UIs may share one server: every client receives every patch, replies only go to
 * the requester.
 */
import type {
  ClientHello,
  ClientMessage,
  Command,
  Event,
  HelloRejection,
  MeterFrame,
  PlayheadFrame,
  Project,
  ReplyValue,
  ServerHello,
  ServerInfo,
  ServerMessage,
} from "@/generated";
import { CommandFailedError, Emitter, type EngineTransport, type SendOptions, type Unsubscribe } from "../EngineTransport";
import { newProjectId } from "../ids";
import { bytesToBase64, decodeBinaryFrame, decodePeaksPayload, encodeBinaryFrame, PROTOCOL_VERSION } from "./binaryFrame";

/** Minimal WebSocket surface (the browser's `WebSocket`; injectable for tests). */
export interface WebSocketLike {
  binaryType: string;
  readonly readyState: number;
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: { code: number; reason: string }) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  send(data: string | ArrayBufferLike | Uint8Array): void;
  close(code?: number, reason?: string): void;
}

export type WebSocketFactory = (url: string) => WebSocketLike;

export interface WsTransportOptions {
  /** Shared secret of the server (`ClientHello::token`). */
  token?: string | null;
  /** Client description for the server log. */
  client?: string;
  /** Name of the project created when the server's store is empty (default "Untitled"). */
  untitledName?: string;
  /** Opens the socket (default: `new WebSocket(url)`). */
  createSocket?: WebSocketFactory;
  /** How long `open()` waits for the hello answer (ms, default 10 s). */
  helloTimeoutMs?: number;
}

/** The server refused the hello (`ServerHello::Rejected`). */
export class RemoteRejectedError extends Error {
  readonly reason: HelloRejection;
  constructor(reason: HelloRejection, message: string) {
    super(message);
    this.name = "RemoteRejectedError";
    this.reason = reason;
  }
}

/** Connection state as seen by the connect dialog. */
export type WsState = "idle" | "opening" | "open" | "closed";

interface Pending {
  resolve: (value: ReplyValue) => void;
  reject: (error: unknown) => void;
  command: Command;
}

const OPEN = 1;

export class WsTransport implements EngineTransport {
  readonly kind = "remote" as const;
  readonly url: string;
  readonly options: WsTransportOptions;

  /** The server's description, once the hello was accepted. */
  info: ServerInfo | null = null;
  /** Per-connection session id from the server (logs). */
  session: string | null = null;
  state: WsState = "idle";

  private socket: WebSocketLike | null = null;
  private opening: Promise<ServerInfo> | null = null;
  private readonly events = new Emitter<Event>();
  private readonly playheadEmitter = new Emitter<PlayheadFrame>();
  private readonly meterEmitter = new Emitter<MeterFrame>();
  private readonly closeEmitter = new Emitter<{ code: number; reason: string }>();
  private readonly pending = new Map<number, Pending>();
  private nextId = 1;
  private disposed = false;

  constructor(url: string, opts: WsTransportOptions = {}) {
    this.url = url;
    this.options = opts;
  }

  /** Open the socket and complete the hello. Idempotent; rejects with `RemoteRejectedError`. */
  open(): Promise<ServerInfo> {
    if (this.disposed) return Promise.reject(new Error("WsTransport disposed"));
    this.opening ??= this.doOpen();
    return this.opening;
  }

  private doOpen(): Promise<ServerInfo> {
    this.state = "opening";
    return new Promise<ServerInfo>((resolve, reject) => {
      let settled = false;
      let opened: WebSocketLike | null = null;
      const fail = (e: Error) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        this.state = "closed";
        try {
          opened?.close();
        } catch {
          // already closed
        }
        reject(e);
      };
      const timer = setTimeout(() => fail(new Error("the server did not answer")), this.options.helloTimeoutMs ?? 10_000);
      let socket: WebSocketLike;
      try {
        socket = (this.options.createSocket ?? ((url) => new WebSocket(url) as unknown as WebSocketLike))(this.url);
      } catch (e) {
        clearTimeout(timer);
        this.state = "closed";
        reject(e instanceof Error ? e : new Error(String(e)));
        return;
      }
      opened = socket;
      this.socket = socket;
      socket.binaryType = "arraybuffer";
      socket.onopen = () => {
        const hello: ClientHello = {
          protocol_version: PROTOCOL_VERSION,
          token: this.options.token || null,
          client: this.options.client ?? clientDescription(),
        };
        socket.send(JSON.stringify(hello));
      };
      socket.onerror = () => fail(new Error(`could not connect to ${this.url}`));
      socket.onclose = (ev) => {
        if (!settled) {
          fail(new Error(`connection closed${ev.reason ? `: ${ev.reason}` : ""} (${ev.code})`));
          return;
        }
        this.onClosed(ev);
      };
      socket.onmessage = (ev) => {
        if (settled) {
          this.onFrame(ev.data);
          return;
        }
        if (typeof ev.data !== "string") return fail(new Error("unexpected binary frame before the hello"));
        let hello: ServerHello;
        try {
          hello = JSON.parse(ev.data) as ServerHello;
        } catch {
          return fail(new Error("invalid hello from the server"));
        }
        if (hello.type === "Rejected") return fail(new RemoteRejectedError(hello.reason, hello.message));
        settled = true;
        clearTimeout(timer);
        this.info = hello.server;
        this.session = hello.session;
        this.state = "open";
        resolve(hello.server);
      };
    });
  }

  async connect(): Promise<Project> {
    await this.open();
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
      : await this.send({ domain: "Project", command: { type: "Create", id: newProjectId(), name: this.options.untitledName ?? "Untitled" } });
    return projectOf(reply);
  }

  send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    return this.request(command, opts, (message) => JSON.stringify(message));
  }

  /**
   * `Media::UploadChunk` with raw bytes in a binary frame (no base64). Same reply and
   * errors as `send`.
   */
  uploadChunk(upload: string, offset: number, bytes: Uint8Array): Promise<ReplyValue> {
    const command: Command = { domain: "Media", command: { type: "UploadChunk", upload, offset, data: "" } };
    return this.request(command, undefined, (message) => encodeBinaryFrame("Bytes", JSON.stringify(message), bytes));
  }

  private request(command: Command, opts: SendOptions | undefined, encode: (m: ClientMessage) => string | Uint8Array): Promise<ReplyValue> {
    const socket = this.socket;
    if (this.disposed || this.state !== "open" || !socket || socket.readyState !== OPEN) {
      return Promise.reject(new CommandFailedError({ code: "InvalidState", message: "not connected to the remote engine" }, command));
    }
    const id = this.nextId++;
    const message: ClientMessage = { id, gesture: opts?.gesture ?? null, command };
    return new Promise<ReplyValue>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, command });
      try {
        socket.send(encode(message));
      } catch (e) {
        this.pending.delete(id);
        reject(new CommandFailedError({ code: "Internal", message: `socket: ${String(e)}` }, command));
      }
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

  /** The connection dropped after the hello (server stopped, network lost). Not called on `dispose`. */
  onClose(listener: (ev: { code: number; reason: string }) => void): Unsubscribe {
    return this.closeEmitter.on(listener);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.state = "closed";
    const socket = this.socket;
    this.socket = null;
    if (socket) {
      socket.onclose = null;
      socket.onmessage = null;
      socket.onerror = null;
      try {
        socket.close(1000, "bye");
      } catch {
        // already closed
      }
    }
    this.rejectPending("transport disposed");
    this.events.clear();
    this.playheadEmitter.clear();
    this.meterEmitter.clear();
    this.closeEmitter.clear();
  }

  private onClosed(ev: { code: number; reason: string }): void {
    if (this.disposed) return;
    this.state = "closed";
    this.socket = null;
    this.rejectPending("connection to the remote engine lost");
    this.closeEmitter.emit({ code: ev.code, reason: ev.reason });
  }

  private rejectPending(message: string): void {
    for (const [, p] of this.pending) p.reject(new CommandFailedError({ code: "InvalidState", message }, p.command));
    this.pending.clear();
  }

  private onFrame(data: unknown): void {
    let m: ServerMessage;
    try {
      m = typeof data === "string" ? (JSON.parse(data) as ServerMessage) : fromBinary(toBytes(data));
    } catch (e) {
      console.error("WsTransport: malformed server frame", e);
      return;
    }
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

function toBytes(data: unknown): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
  throw new Error("unsupported binary frame type (set binaryType = 'arraybuffer')");
}

/** Rebuild the plain JSON message from a binary frame. */
export function fromBinary(frame: Uint8Array): ServerMessage {
  const { kind, headerJson, payload } = decodeBinaryFrame(frame);
  const m = JSON.parse(headerJson) as ServerMessage;
  if (m.kind !== "Reply" || m.body.result.status !== "Ok") throw new Error("binary frames carry Ok replies");
  const value = m.body.result.value;
  if (kind === "Bytes" && value.type === "Bytes") {
    value.chunk.data = bytesToBase64(payload);
  } else if (kind === "Peaks" && value.type === "Peaks") {
    const { min, max } = decodePeaksPayload(payload, value.peaks.min.length);
    value.peaks.min = min;
    value.peaks.max = max;
  } else {
    throw new Error(`binary ${kind} frame for a ${value.type} reply`);
  }
  return m;
}

function projectOf(reply: ReplyValue): Project {
  if (reply.type !== "Project") {
    throw new CommandFailedError({ code: "Internal", message: `expected a Project reply, got ${reply.type}` });
  }
  return reply.project;
}

function clientDescription(): string {
  const ua = typeof navigator !== "undefined" ? navigator.userAgent : "unknown";
  return `Ethereal web / ${ua}`;
}
