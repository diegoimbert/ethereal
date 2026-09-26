import { describe, expect, it, vi } from "vitest";
import type { ClientMessage, Event, MeterFrame, PlayheadFrame, Project, ServerMessage } from "@/generated";
import { createEmptyProject } from "../mock/demoProject";
import { CommandFailedError } from "../EngineTransport";
import { newId, newProjectId } from "../ids";
import { type ChannelLike, type InvokeFn, TauriTransport } from "./TauriTransport";

class FakeChannel<T> implements ChannelLike<T> {
  onmessage: (message: T) => void = () => {};
}

interface Channels {
  messages: FakeChannel<ServerMessage>;
  playhead: FakeChannel<PlayheadFrame>;
  meters: FakeChannel<MeterFrame>;
}

const INFO = {
  version: "0.0.1",
  instance: "test",
  data_dir: "/d",
  projects_root: "/d/projects",
  backend: "null",
  sample_rate: 48000,
  buffer_size: 256,
  device: null,
};

/**
 * A fake `invoke` backed by a tiny engine: `handle(message, emit)` returns what to send
 * back on the `messages` channel (asynchronously, like real IPC).
 */
function fakeHost(handle: (m: ClientMessage) => ServerMessage[]) {
  const state: { channels: Channels | null; sent: ClientMessage[]; calls: string[] } = {
    channels: null,
    sent: [],
    calls: [],
  };
  const invoke: InvokeFn = vi.fn((cmd: string, args?: Record<string, unknown>) => {
    state.calls.push(cmd);
    switch (cmd) {
      case "ether_connect":
        state.channels = args as unknown as Channels;
        return Promise.resolve(INFO);
      case "ether_send": {
        const message = (args as { message: ClientMessage }).message;
        state.sent.push(message);
        const out = handle(message);
        setTimeout(() => {
          for (const m of out) state.channels?.messages.onmessage(m);
        }, 0);
        return Promise.resolve(null);
      }
      case "ether_disconnect":
        return Promise.resolve(null);
      default:
        return Promise.reject(new Error(`unknown command ${cmd}`));
    }
  });
  return { invoke, state };
}

const ok = (id: number, value: object): ServerMessage =>
  ({ kind: "Reply", body: { id, result: { status: "Ok", value } } }) as ServerMessage;
const err = (id: number, code: string, message = "x"): ServerMessage =>
  ({ kind: "Reply", body: { id, result: { status: "Err", error: { code, message } } } }) as ServerMessage;

function transport(handle: (m: ClientMessage) => ServerMessage[]) {
  const host = fakeHost(handle);
  const t = new TauriTransport({ invoke: host.invoke, createChannel: () => new FakeChannel() });
  return { t, host };
}

describe("TauriTransport", () => {
  const project: Project = createEmptyProject(newId, "Song", newProjectId());

  it("connects, registers channels and returns the open project", async () => {
    const { t, host } = transport((m) => [ok(m.id, { type: "Project", project })]);
    const p = await t.connect();
    expect(p.settings.name).toBe("Song");
    expect(host.state.calls).toEqual(["ether_connect", "ether_send"]);
    expect(host.state.sent[0]?.command).toEqual({ domain: "Project", command: { type: "Get" } });
    expect(t.info?.backend).toBe("null");
  });

  it("opens the newest stored project when none is open", async () => {
    const { t, host } = transport((m) => {
      const c = m.command.command as { type: string };
      if (c.type === "Get") return [err(m.id, "InvalidState")];
      if (c.type === "List")
        return [
          ok(m.id, {
            type: "Projects",
            projects: [
              { id: "old", name: "Old", modified_ms: 1 },
              { id: "new", name: "New", modified_ms: 5 },
            ],
          }),
        ];
      return [ok(m.id, { type: "Project", project })];
    });
    await t.connect();
    expect(host.state.sent.map((m) => m.command.command)).toEqual([
      { type: "Get" },
      { type: "List" },
      { type: "Open", id: "new" },
    ]);
  });

  it("creates an Untitled project when the store is empty", async () => {
    const { t, host } = transport((m) => {
      const c = m.command.command as { type: string };
      if (c.type === "Get") return [err(m.id, "InvalidState")];
      if (c.type === "List") return [ok(m.id, { type: "Projects", projects: [] })];
      return [ok(m.id, { type: "Project", project })];
    });
    await t.connect();
    const create = host.state.sent[2]!.command.command as { type: string; id: string; name: string };
    expect(create.type).toBe("Create");
    expect(create.name).toBe("Untitled");
    expect(create.id).toMatch(/^[0-9a-f-]{36}$/);
  });

  it("delivers patches before resolving send, with gesture and unique ids", async () => {
    const patch: Event = {
      type: "Notification",
      level: "Info",
      message: "patch-stand-in",
    };
    const { t, host } = transport((m) => {
      if ((m.command.command as { type: string }).type === "Get") return [ok(m.id, { type: "Project", project })];
      return [{ kind: "Event", body: patch }, ok(m.id, { type: "Unit" })];
    });
    await t.connect();
    const seen: Event[] = [];
    t.onEvent((e) => seen.push(e));
    const a = t.send({ domain: "Transport", command: { type: "Play" } }, { gesture: 7 as never });
    const b = t.send({ domain: "Transport", command: { type: "Stop" } });
    await expect(a).resolves.toEqual({ type: "Unit" });
    expect(seen).toEqual([patch]);
    await b;
    expect(seen.length).toBe(2);
    const [, first, second] = host.state.sent as [ClientMessage, ClientMessage, ClientMessage];
    expect(first.gesture).toBe(7);
    expect(second.gesture).toBeNull();
    expect(first.id).not.toBe(second.id);
  });

  it("rejects with CommandFailedError on Err replies and on IPC failure", async () => {
    const { t } = transport((m) => {
      if ((m.command.command as { type: string }).type === "Get") return [ok(m.id, { type: "Project", project })];
      return [err(m.id, "NotFound", "no such track")];
    });
    await t.connect();
    const cmd = { domain: "Transport", command: { type: "Play" } } as const;
    const e = await t.send(cmd).catch((x: unknown) => x);
    expect(e).toBeInstanceOf(CommandFailedError);
    expect((e as CommandFailedError).code).toBe("NotFound");
    expect((e as CommandFailedError).command).toEqual(cmd);

    const broken = new TauriTransport({
      invoke: () => Promise.reject(new Error("boom")),
      createChannel: () => new FakeChannel(),
    });
    const e2 = await broken.send(cmd).catch((x: unknown) => x);
    expect((e2 as CommandFailedError).code).toBe("Internal");
  });

  it("routes playhead and meter channels to their subscribers", async () => {
    const { t, host } = transport((m) => [ok(m.id, { type: "Project", project })]);
    await t.connect();
    const ph: PlayheadFrame[] = [];
    const mt: MeterFrame[] = [];
    const unsub = t.subscribePlayhead((f) => ph.push(f));
    t.subscribeMeters((f) => mt.push(f));
    const frame: PlayheadFrame = {
      transport: { position: 1.5, seconds: 0.75, playing: true, bpm: 120 },
      session: [],
    } as unknown as PlayheadFrame;
    host.state.channels!.playhead.onmessage(frame);
    host.state.channels!.meters.onmessage({ tracks: [], cpu_load: 0.1 });
    // Streams multiplexed on the message channel are routed too.
    host.state.channels!.messages.onmessage({ kind: "Playhead", body: frame });
    expect(ph).toEqual([frame, frame]);
    expect(mt).toEqual([{ tracks: [], cpu_load: 0.1 }]);
    unsub();
    host.state.channels!.playhead.onmessage(frame);
    expect(ph.length).toBe(2);
  });

  it("dispose rejects pending sends and disconnects", async () => {
    const { t, host } = transport((m) =>
      (m.command.command as { type: string }).type === "Get" ? [ok(m.id, { type: "Project", project })] : [],
    );
    await t.connect();
    const p = t.send({ domain: "Transport", command: { type: "Play" } });
    t.dispose();
    await expect(p).rejects.toMatchObject({ code: "InvalidState" });
    await expect(t.send({ domain: "Transport", command: { type: "Play" } })).rejects.toBeInstanceOf(
      CommandFailedError,
    );
    expect(host.state.calls).toContain("ether_disconnect");
    await expect(t.connect()).rejects.toThrow("disposed");
  });
});
