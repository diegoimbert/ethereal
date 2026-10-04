import { describe, expect, it, vi } from "vitest";
import type { ClientHello, ClientMessage, Event, Project, ReplyValue, ServerHello, ServerInfo, ServerMessage } from "@/generated";
import { isCommandFailed } from "../EngineTransport";
import { createEmptyProject } from "../mock/demoProject";
import { decodeBinaryFrame, encodeBinaryFrame } from "./binaryFrame";
import { RemoteRejectedError, WsTransport, type WebSocketLike } from "./WsTransport";

let counter = 0;
const nextId = () => `01J0000000000000000000${String(counter++).padStart(4, "0")}`;
const project = (name = "Remote"): Project => createEmptyProject(nextId, name, "01890a5d-ac96-774b-bcce-b302099a8057");

const INFO: ServerInfo = {
  name: "studio",
  app_version: "0.1.0",
  protocol_version: 1,
  instance: "test",
  auth_required: true,
  capabilities: { plugins: true, recording: true, upload: true, export: true, collab: false },
};

type Handler = (msg: ClientMessage) => (ServerMessage | Uint8Array)[];

/** A fake server socket: answers the hello, then runs `handler` for every message. */
class FakeSocket implements WebSocketLike {
  binaryType = "blob";
  readyState = 0;
  onopen: ((ev: unknown) => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: ((ev: { code: number; reason: string }) => void) | null = null;
  onerror: ((ev: unknown) => void) | null = null;
  hello: ClientHello | null = null;
  readonly sent: (ClientMessage | { binary: Uint8Array })[] = [];
  closed: number | null = null;

  constructor(
    private answer: (hello: ClientHello) => ServerHello,
    private handler: Handler = () => [],
  ) {
    setTimeout(() => {
      this.readyState = 1;
      this.onopen?.({});
    }, 0);
  }

  send(data: string | ArrayBufferLike | Uint8Array): void {
    if (typeof data === "string" && !this.hello) {
      this.hello = JSON.parse(data) as ClientHello;
      const a = this.answer(this.hello);
      setTimeout(() => {
        this.onmessage?.({ data: JSON.stringify(a) });
        if (a.type === "Rejected") this.serverClose(4001, a.message);
      }, 0);
      return;
    }
    let msg: ClientMessage;
    if (typeof data === "string") {
      msg = JSON.parse(data) as ClientMessage;
      this.sent.push(msg);
    } else {
      const bytes = data instanceof Uint8Array ? data : new Uint8Array(data as ArrayBuffer);
      const f = decodeBinaryFrame(bytes);
      msg = JSON.parse(f.headerJson) as ClientMessage;
      this.sent.push({ binary: f.payload.slice() });
    }
    const out = this.handler(msg);
    setTimeout(() => this.push(out), 0);
  }

  push(out: (ServerMessage | Uint8Array)[]): void {
    for (const m of out) {
      this.onmessage?.({ data: m instanceof Uint8Array ? m.buffer.slice(m.byteOffset, m.byteOffset + m.byteLength) : JSON.stringify(m) });
    }
  }

  serverClose(code: number, reason = ""): void {
    this.readyState = 3;
    this.onclose?.({ code, reason });
  }

  close(code?: number): void {
    this.closed = code ?? 1005;
    this.readyState = 3;
  }
}

const welcome = (): ServerHello => ({ type: "Welcome", server: INFO, session: "abc" });
const ok = (id: number, value: ReplyValue): ServerMessage => ({ kind: "Reply", body: { id, result: { status: "Ok", value } } });

function setup(handler: Handler = () => [], answer: (h: ClientHello) => ServerHello = welcome) {
  let socket: FakeSocket | null = null;
  const t = new WsTransport("ws://studio:1234/", {
    token: "tok",
    client: "test",
    createSocket: () => (socket = new FakeSocket(answer, handler)),
  });
  return { t, socket: () => socket! };
}

describe("WsTransport", () => {
  it("says hello with the token and protocol version", async () => {
    const { t, socket } = setup();
    expect(t.kind).toBe("remote");
    await expect(t.open()).resolves.toEqual(INFO);
    expect(socket().hello).toEqual({ protocol_version: 1, token: "tok", client: "test" });
    expect(socket().binaryType).toBe("arraybuffer");
    expect(t.state).toBe("open");
    expect(t.session).toBe("abc");
    // Idempotent.
    await t.open();
  });

  it("reports rejections", async () => {
    const { t } = setup(undefined, () => ({ type: "Rejected", reason: "BadToken", message: "invalid token" }));
    const e: unknown = await t.open().catch((x: unknown) => x);
    expect(e).toBeInstanceOf(RemoteRejectedError);
    expect((e as RemoteRejectedError).reason).toBe("BadToken");
    expect(t.state).toBe("closed");
    await expect(t.send({ domain: "Transport", command: { type: "Play" } })).rejects.toSatisfy((x) => isCommandFailed(x, "InvalidState"));
  });

  it("connects to the server's open project", async () => {
    const p = project();
    const { t } = setup((m) => (m.command.domain === "Project" ? [ok(m.id, { type: "Project", project: p })] : []));
    expect((await t.connect())?.settings.name).toBe("Remote");
  });

  it("connects: opens nothing when the server has no open project (base-131)", async () => {
    const { t, socket } = setup((m) => {
      const c = m.command;
      if (c.domain === "Project" && c.command.type === "Get")
        return [{ kind: "Reply", body: { id: m.id, result: { status: "Err", error: { code: "InvalidState", message: "no project" } } } }];
      return [];
    });
    await expect(t.connect()).resolves.toBeNull();
    expect(socket().sent.map((m) => ("command" in m ? m.command.command.type : "binary"))).toEqual(["Get"]);
  });

  it("delivers patches before the reply, and streams", async () => {
    const patch: Event = { type: "Patch", patch: { revision: 1, changes: [], history: { can_undo: true, can_redo: false, undo_label: null, redo_label: null } } };
    const { t, socket } = setup((m) => [{ kind: "Event", body: patch }, ok(m.id, { type: "Unit" })]);
    await t.open();
    const log: string[] = [];
    t.onEvent((e) => log.push(e.type));
    const playhead = vi.fn();
    t.subscribePlayhead(playhead);
    await t.send({ domain: "Transport", command: { type: "Play" } }, { gesture: 3 });
    log.push("reply");
    expect(log).toEqual(["Patch", "reply"]);
    expect(socket().sent[0]).toMatchObject({ id: 1, gesture: 3 });
    socket().push([{ kind: "Playhead", body: { transport: { playing: true } } } as unknown as ServerMessage]);
    expect(playhead).toHaveBeenCalledTimes(1);
  });

  it("rebuilds binary replies (peaks, bytes) into their JSON shapes", async () => {
    const { t } = setup((m) => {
      if (m.command.domain === "Media") {
        const header = JSON.stringify(
          ok(m.id, { type: "Peaks", peaks: { media: "01J8ZK3V9Q4N2B7XK6M1T0R5AS", samples_per_peak: 256, start_frame: 0, min: [[]], max: [[]] } }),
        );
        const payload = new Uint8Array(new Float32Array([-0.5, -0.25, 0.5, 1]).buffer);
        return [encodeBinaryFrame("Peaks", header, payload)];
      }
      const header = JSON.stringify(ok(m.id, { type: "Bytes", chunk: { offset: 0, data: "", eof: true } }));
      return [encodeBinaryFrame("Bytes", header, new Uint8Array([1, 2, 3]))];
    });
    await t.open();
    const peaks = await t.send({ domain: "Media", command: { type: "GetPeaks", request: { media: "x", samples_per_peak: 256, start_frame: 0, frame_count: 512 } } });
    expect(peaks).toMatchObject({ type: "Peaks", peaks: { min: [[-0.5, -0.25]], max: [[0.5, 1]] } });
    const bytes = await t.send({ domain: "Export", command: { type: "ReadChunk", download: "d", offset: 0, max_bytes: 10 } } as never);
    expect(bytes).toMatchObject({ type: "Bytes", chunk: { data: "AQID", eof: true } });
  });

  it("sends upload chunks as binary frames", async () => {
    const { t, socket } = setup((m) => [ok(m.id, { type: "Unit" })]);
    await t.open();
    await t.uploadChunk("u1", 4, new Uint8Array([7, 8, 9]));
    expect(socket().sent[0]).toEqual({ binary: new Uint8Array([7, 8, 9]) });
  });

  it("rejects pending requests and notifies when the connection drops", async () => {
    const { t, socket } = setup(() => []);
    await t.open();
    const closed = vi.fn();
    t.onClose(closed);
    const pending = t.send({ domain: "Transport", command: { type: "Play" } });
    socket().serverClose(1006);
    await expect(pending).rejects.toSatisfy((x) => isCommandFailed(x, "InvalidState"));
    expect(closed).toHaveBeenCalledWith({ code: 1006, reason: "" });
    expect(t.state).toBe("closed");
  });

  it("disposes: closes the socket, rejects pending, no close notification", async () => {
    const { t, socket } = setup(() => []);
    await t.open();
    const closed = vi.fn();
    t.onClose(closed);
    const pending = t.send({ domain: "Transport", command: { type: "Play" } });
    t.dispose();
    await expect(pending).rejects.toSatisfy((x) => isCommandFailed(x, "InvalidState"));
    expect(socket().closed).toBe(1000);
    expect(closed).not.toHaveBeenCalled();
  });

  it("fails when the server never answers", async () => {
    vi.useFakeTimers();
    try {
      const t = new WsTransport("ws://x/", { createSocket: () => ({ binaryType: "", readyState: 0, onopen: null, onmessage: null, onclose: null, onerror: null, send() {}, close() {} }), helloTimeoutMs: 50 });
      const p = t.open();
      vi.advanceTimersByTime(60);
      await expect(p).rejects.toThrow(/did not answer/);
    } finally {
      vi.useRealTimers();
    }
  });
});
