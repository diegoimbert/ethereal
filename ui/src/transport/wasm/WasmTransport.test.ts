import { describe, expect, it, vi } from "vitest";
import type { ClientMessage, Event, Project, ReplyValue, ServerMessage } from "@/generated";
import { CommandFailedError, Emitter, isCommandFailed } from "../EngineTransport";
import { createEmptyProject } from "../mock/demoProject";
import { WasmTransport, type WasmEndpoint } from "./WasmTransport";

let counter = 0;
const nextId = () => `01J0000000000000000000${String(counter++).padStart(4, "0")}`;
const PROJECT_ID = "01890a5d-ac96-774b-bcce-b302099a8057";
const project = (name = "Demo"): Project => createEmptyProject(nextId, name, PROJECT_ID);

type Handler = (msg: ClientMessage) => ServerMessage[];

/** Endpoint whose "Worker" is a synchronous handler; batches arrive asynchronously. */
class FakeEndpoint implements WasmEndpoint {
  readonly sent: ClientMessage[] = [];
  readonly batches = new Emitter<string>();
  readonly fatal = new Emitter<Error>();
  started = 0;
  disposed = false;
  constructor(
    private handler: Handler,
    private boot: () => Promise<void> = () => Promise.resolve(),
  ) {}
  start(): Promise<void> {
    this.started++;
    return this.boot();
  }
  post(json: string): void {
    const msg = JSON.parse(json) as ClientMessage;
    this.sent.push(msg);
    const out = this.handler(msg);
    setTimeout(() => this.batches.emit(JSON.stringify(out)), 0);
  }
  onMessages(l: (batch: string) => void) {
    return this.batches.on(l);
  }
  onFatal(l: (e: Error) => void) {
    return this.fatal.on(l);
  }
  dispose(): void {
    this.disposed = true;
  }
  /** Push a batch as if from the Worker's tick. */
  push(messages: ServerMessage[]): void {
    this.batches.emit(JSON.stringify(messages));
  }
}

const ok = (id: number, value: ReplyValue): ServerMessage => ({ kind: "Reply", body: { id, result: { status: "Ok", value } } });
const fail = (id: number, code: "InvalidState" | "NotFound", message = "nope"): ServerMessage => ({
  kind: "Reply",
  body: { id, result: { status: "Err", error: { code, message } } },
});

describe("WasmTransport", () => {
  it("connects to the open project", async () => {
    const p = project();
    const ep = new FakeEndpoint((m) => [ok(m.id, { type: "Project", project: p })]);
    const t = new WasmTransport({ endpoint: ep });
    await expect(t.connect()).resolves.toEqual(p);
    expect(ep.started).toBe(1);
    expect(ep.sent.map((m) => m.command)).toEqual([{ domain: "Project", command: { type: "Get" } }]);
  });

  it("creates a project when none is open and the store is empty", async () => {
    const created = project("Untitled");
    const ep = new FakeEndpoint((m) => {
      const c = m.command;
      if (c.domain !== "Project") throw new Error("unexpected");
      switch (c.command.type) {
        case "Get":
          return [fail(m.id, "InvalidState")];
        case "List":
          return [ok(m.id, { type: "Projects", projects: [] })];
        case "Create":
          return [{ kind: "Event", body: { type: "ProjectLoaded", project: created } }, ok(m.id, { type: "Project", project: created })];
        default:
          throw new Error("unexpected");
      }
    });
    const t = new WasmTransport({ endpoint: ep });
    const events: Event[] = [];
    t.onEvent((e) => events.push(e));
    await expect(t.connect()).resolves.toEqual(created);
    const create = ep.sent[2]!.command;
    expect(create.domain === "Project" && create.command.type === "Create" && create.command.name).toBe("Untitled");
    expect(events.map((e) => e.type)).toEqual(["ProjectLoaded"]);
  });

  it("opens the newest stored project when none is open", async () => {
    const p = project("Newest");
    const ep = new FakeEndpoint((m) => {
      const c = m.command;
      if (c.domain !== "Project") throw new Error("unexpected");
      if (c.command.type === "Get") return [fail(m.id, "InvalidState")];
      if (c.command.type === "List")
        return [
          ok(m.id, {
            type: "Projects",
            projects: [
              { id: PROJECT_ID, name: "Newest", modified_ms: 2 },
              { id: "01890a5d-ac96-774b-bcce-b302099a8058", name: "Old", modified_ms: 1 },
            ],
          }),
        ];
      if (c.command.type === "Open" && c.command.id === PROJECT_ID) return [ok(m.id, { type: "Project", project: p })];
      throw new Error("unexpected");
    });
    await expect(new WasmTransport({ endpoint: ep }).connect()).resolves.toEqual(p);
  });

  it("delivers a command's patches before resolving it", async () => {
    const p = project();
    const order: string[] = [];
    const ep = new FakeEndpoint((m) => {
      if (m.command.domain === "Project") return [ok(m.id, { type: "Project", project: p })];
      return [
        {
          kind: "Event",
          body: { type: "Patch", patch: { revision: 1, changes: [], history: { can_undo: true, can_redo: false, undo_label: null, redo_label: null } } },
        },
        ok(m.id, { type: "Unit" }),
      ];
    });
    const t = new WasmTransport({ endpoint: ep });
    t.onEvent((e) => order.push(e.type));
    await t.connect();
    await t.send({ domain: "Transport", command: { type: "SetTempo", bpm: 90 } }, { gesture: 7 });
    order.push("resolved");
    expect(order).toEqual(["Patch", "resolved"]);
    expect(ep.sent[1]!.gesture).toBe(7);
    expect(ep.sent[0]!.gesture).toBeNull();
  });

  it("rejects failed commands with CommandFailedError", async () => {
    const ep = new FakeEndpoint((m) => [fail(m.id, "NotFound", "no such track")]);
    const t = new WasmTransport({ endpoint: ep });
    const cmd = { domain: "Track", command: { type: "Delete", track: "01J00000000000000000000000" } } as const;
    const err: unknown = await t.send(cmd as never).catch((e: unknown) => e);
    expect(isCommandFailed(err, "NotFound")).toBe(true);
    expect((err as CommandFailedError).message).toContain("no such track");
    expect((err as CommandFailedError).command).toEqual(cmd);
  });

  it("routes playhead and meter streams", async () => {
    const ep = new FakeEndpoint((m) => [ok(m.id, { type: "Project", project: project() })]);
    const t = new WasmTransport({ endpoint: ep });
    const playhead = vi.fn();
    const meters = vi.fn();
    t.subscribePlayhead(playhead);
    t.subscribeMeters(meters);
    await t.connect();
    const frame = { transport: { position: 1.5, seconds: 0.75, playing: true, bpm: 120 } };
    const meterFrame = { tracks: [{ track: "01J00000000000000000000000", peak: [0.5, 0.5], rms: [0.1, 0.1], clipped: false }], cpu_load: 0.1 };
    ep.push([
      { kind: "Playhead", body: frame },
      { kind: "Meters", body: meterFrame },
    ] as ServerMessage[]);
    expect(playhead).toHaveBeenCalledWith(frame);
    expect(meters).toHaveBeenCalledWith(meterFrame);
  });

  it("fails pending and later commands when the engine dies", async () => {
    const ep = new FakeEndpoint(() => []); // never replies
    const t = new WasmTransport({ endpoint: ep });
    const events: Event[] = [];
    t.onEvent((e) => events.push(e));
    const pending = t.connect();
    await new Promise((r) => setTimeout(r, 0));
    ep.fatal.emit(new Error("worklet crashed"));
    await expect(pending).rejects.toBeInstanceOf(CommandFailedError);
    await expect(t.send({ domain: "Transport", command: { type: "Play" } })).rejects.toThrow("worklet crashed");
    expect(events).toEqual([{ type: "Notification", level: "Error", message: "Audio engine stopped: worklet crashed" }]);
  });

  it("rejects connect when booting fails or no endpoint is configured", async () => {
    await expect(new WasmTransport().connect()).rejects.toThrow("no web engine endpoint");
    const ep = new FakeEndpoint(() => [], () => Promise.reject(new Error("not cross-origin isolated")));
    await expect(new WasmTransport({ endpoint: ep }).connect()).rejects.toThrow("not cross-origin isolated");
  });

  it("waits for boot before posting, and dispose rejects pending commands", async () => {
    let release!: () => void;
    const ep = new FakeEndpoint(() => [], () => new Promise<void>((r) => (release = r)));
    const t = new WasmTransport({ endpoint: ep });
    const connecting = t.connect();
    await new Promise((r) => setTimeout(r, 0));
    expect(ep.sent).toHaveLength(0);
    release();
    await new Promise((r) => setTimeout(r, 0));
    expect(ep.sent).toHaveLength(1);
    t.dispose();
    await expect(connecting).rejects.toBeInstanceOf(CommandFailedError);
    expect(ep.disposed).toBe(true);
    await expect(t.send({ domain: "Transport", command: { type: "Play" } })).rejects.toThrow("disposed");
  });
});
