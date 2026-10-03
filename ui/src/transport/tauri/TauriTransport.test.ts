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
    expect(p?.settings.name).toBe("Song");
    expect(host.state.calls).toEqual(["ether_connect", "ether_send"]);
    expect(host.state.sent[0]?.command).toEqual({ domain: "Project", command: { type: "Get" } });
    expect(t.info?.backend).toBe("null");
  });

  it("opens nothing when no project is open (base-131: no reopen on launch)", async () => {
    const { t, host } = transport((m) => {
      const c = m.command.command as { type: string };
      if (c.type === "Get") return [err(m.id, "InvalidState")];
      return [ok(m.id, { type: "Project", project })];
    });
    await expect(t.connect()).resolves.toBeNull();
    expect(host.state.sent.map((m) => m.command.command)).toEqual([{ type: "Get" }]);
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

  it("serializes ether_send invokes so the host receives commands in call order", async () => {
    // Each invoke stays pending until the test resolves it; the test resolves them
    // newest-first, so without chaining the host would accept 3, 2, 1.
    const accepted: number[] = [];
    const inFlight: Array<{ id: number; accept: () => void }> = [];
    let channels: Channels | null = null;
    const invoke: InvokeFn = (cmd, args) => {
      if (cmd === "ether_connect") {
        channels = args as unknown as Channels;
        return Promise.resolve(INFO);
      }
      const message = (args as { message: ClientMessage }).message;
      return new Promise((resolve) => {
        inFlight.push({
          id: message.id,
          accept: () => {
            accepted.push(message.id);
            resolve(null);
            setTimeout(() => channels?.messages.onmessage(ok(message.id, { type: "Unit" })), 0);
          },
        });
      });
    };
    const t = new TauriTransport({ invoke, createChannel: () => new FakeChannel() });
    // Register channels directly (skip connect's project bootstrap).
    channels = { messages: new FakeChannel(), playhead: new FakeChannel(), meters: new FakeChannel() };
    (t as unknown as { channels: unknown[] }).channels = [];
    channels.messages.onmessage = (m) => (t as unknown as { onServerMessage(m: ServerMessage): void }).onServerMessage(m);
    const flush = () => new Promise((r) => setTimeout(r, 0));
    const sends = [1, 2, 3].map(() => t.send({ domain: "Transport", command: { type: "Play" } }));
    while (accepted.length < 3) {
      await flush();
      // Never more than one invoke in flight.
      expect(inFlight.length).toBeLessThanOrEqual(1);
      inFlight.splice(0).reverse().forEach((f) => f.accept());
    }
    await Promise.all(sends);
    expect(accepted).toEqual([1, 2, 3]);
  });

  it("keeps sending after an ether_send invoke is rejected", async () => {
    let calls = 0;
    const { t, host } = transport((m) => [ok(m.id, { type: "Project", project })]);
    await t.connect();
    const base = host.invoke;
    const flaky: InvokeFn = (cmd, args) => {
      if (cmd === "ether_send" && calls++ === 0) return Promise.reject(new Error("ipc down"));
      return base(cmd, args);
    };
    (t as unknown as { invoke: InvokeFn }).invoke = flaky;
    const first = t.send({ domain: "Transport", command: { type: "Play" } });
    const second = t.send({ domain: "Transport", command: { type: "Stop" } });
    await expect(first).rejects.toMatchObject({ code: "Internal" });
    await expect(second).resolves.toEqual({ type: "Project", project });
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

describe("TauriTransport OS files (file-import)", () => {
  it("opens the OS dialog for audio files and returns the picked paths", async () => {
    const openDialog = vi
      .fn()
      .mockResolvedValueOnce(["/a/Kick.wav", "/b/Loop.flac"])
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce("/c.wav");
    const t = new TauriTransport({ invoke: fakeHost(() => []).invoke, openDialog });
    expect(await t.pickAudioFiles()).toEqual(["/a/Kick.wav", "/b/Loop.flac"]);
    expect(openDialog.mock.calls[0]![0]).toMatchObject({
      multiple: true,
      directory: false,
      filters: [{ extensions: expect.arrayContaining(["wav", "flac", "mp3"]) }],
    });
    expect(await t.pickAudioFiles()).toBeNull();
    expect(await t.pickAudioFiles()).toEqual(["/c.wav"]);
  });

  it("opens the OS folder dialog for the Relink folder search (media-references)", async () => {
    const openDialog = vi.fn().mockResolvedValueOnce("/Volumes/Samples").mockResolvedValueOnce(null);
    const t = new TauriTransport({ invoke: fakeHost(() => []).invoke, openDialog });
    expect(await t.pickFolder()).toBe("/Volumes/Samples");
    expect(openDialog.mock.calls[0]![0]).toMatchObject({ multiple: false, directory: true });
    expect(await t.pickFolder()).toBeNull();
  });

  it("delivers path drops from the shell until unsubscribed", async () => {
    let handler: ((e: { payload: unknown }) => void) | null = null;
    const unlisten = vi.fn();
    const listen = vi.fn((event: string, h: (e: { payload: unknown }) => void) => {
      expect(event).toBe("ether://path-drop");
      handler = h;
      return Promise.resolve(unlisten);
    });
    const t = new TauriTransport({ invoke: fakeHost(() => []).invoke, listen: listen as never });
    const got: string[][] = [];
    const off = t.onPathDrop((p) => got.push(p));
    await Promise.resolve();
    handler!({ payload: { paths: ["/x/a.wav", 3] } });
    handler!({ payload: {} });
    expect(got).toEqual([["/x/a.wav"]]);
    off();
    expect(unlisten).toHaveBeenCalled();
  });
});
