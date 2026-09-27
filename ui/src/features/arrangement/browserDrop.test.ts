import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, Event, GestureId, MediaId, MediaRef, ReplyValue } from "@/generated";
import { useProjectStore } from "@/state";
import {
  cmd,
  CommandFailedError,
  Emitter,
  MockTransport,
  type EngineTransport,
  type SendOptions,
} from "@/transport";
import { dropBrowserMedia, type BrowserDragPayload } from "./browserDrop";
import { resetArrangementUi, useArrangementUi } from "./uiStore";

const kick: BrowserDragPayload = {
  version: 1,
  kind: "media",
  name: "Kick.wav",
  file_kind: "Audio",
  source: { type: "Location", location: { type: "Library", id: "library" }, path: "Drums/Kick.wav" },
};

const store = () => useProjectStore.getState();
const project = () => store().project!;
const drums = () => Object.values(project().tracks).find((t) => t.name === "Drums")!.id;

/** Every command a transport receives, with its gesture. */
type Sent = Array<{ command: Command; gesture: GestureId | undefined }>;

function createAudioMedia(c: Command): MediaId[] {
  if (c.domain === "Clip" && c.command.type === "CreateAudio") return [c.command.media];
  if (c.domain === "Edit" && c.command.type === "Batch") return c.command.commands.flatMap(createAudioMedia);
  return [];
}

/**
 * The mock engine, but imports have an unknown length (like a VBR MP3 without a Xing
 * header): the import reply and patch say `frames: 0`, `CreateAudio` from that media fails
 * with `InvalidState`, until `decode(media)` patches the real length in (as the
 * controller's `fill_media_length` does). Patch revisions are renumbered to make room for
 * that extra patch.
 */
class VbrTransport implements EngineTransport {
  readonly kind = "mock";
  readonly sent: Sent = [];
  private events = new Emitter<Event>();
  private loading = new Set<MediaId>();
  private offset = 0;
  private revision = 0;

  constructor(readonly inner: MockTransport) {
    inner.onEvent((e) => {
      if (e.type !== "Patch") return this.events.emit(e);
      const changes = e.patch.changes.map((c) =>
        c.type === "Upsert" && c.entity.type === "Media" && this.loading.has(c.entity.value.id)
          ? { ...c, entity: { type: "Media" as const, value: { ...c.entity.value, frames: 0 } } }
          : c,
      );
      this.revision = e.patch.revision + this.offset;
      this.events.emit({ type: "Patch", patch: { ...e.patch, revision: this.revision, changes } });
    });
  }

  connect() {
    return this.inner.connect();
  }

  async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push({ command, gesture: opts?.gesture });
    if (createAudioMedia(command).some((m) => this.loading.has(m))) {
      throw new CommandFailedError({ code: "InvalidState", message: "the media is still loading" }, command);
    }
    if (command.domain === "Media" && command.command.type === "Import") this.loading.add(command.command.id);
    const reply = await this.inner.send(command, opts);
    return reply.type === "Media" && this.loading.has(reply.media.id) ? { ...reply, media: { ...reply.media, frames: 0 } } : reply;
  }

  /** Background decode done: patch the media's real length in. */
  decode(media: MediaId): MediaRef {
    this.loading.delete(media);
    const value = this.inner.snapshot().media[media]!;
    this.offset++;
    this.revision++;
    this.events.emit({
      type: "Patch",
      patch: { revision: this.revision, changes: [{ type: "Upsert", entity: { type: "Media", value } }], history: store().history },
    });
    return value;
  }

  emit(e: Event) {
    this.events.emit(e);
  }

  onEvent(listener: (event: Event) => void) {
    return this.events.on(listener);
  }
  subscribePlayhead(l: Parameters<EngineTransport["subscribePlayhead"]>[0]) {
    return this.inner.subscribePlayhead(l);
  }
  subscribeMeters(l: Parameters<EngineTransport["subscribeMeters"]>[0]) {
    return this.inner.subscribeMeters(l);
  }
  dispose() {
    this.inner.dispose();
  }
}

/** Record the commands of a plain mock. */
function recording(mock: MockTransport): Sent {
  const sent: Sent = [];
  const send = mock.send.bind(mock);
  mock.send = (command, opts) => {
    sent.push({ command, gesture: opts?.gesture });
    return send(command, opts);
  };
  return sent;
}

async function connect(t: EngineTransport) {
  t.onEvent((e) => {
    if (e.type === "Patch") store().applyPatch(e.patch);
  });
  store().loadProject(await t.connect());
}

async function flush() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

async function undo(t: EngineTransport) {
  await t.send(cmd("Edit", { type: "Undo" }));
}

const counts = () => ({
  tracks: Object.keys(project().tracks).length,
  clips: Object.keys(project().clips).length,
  media: Object.keys(project().media).length,
});

/** Gestures of the edits (not `EndGesture`) and whether `EndGesture` closed them. */
function gestures(sent: Sent) {
  const edits = sent.filter((s) => !(s.command.domain === "Edit" && s.command.command.type === "EndGesture"));
  const ends = sent.flatMap((s) => (s.command.domain === "Edit" && s.command.command.type === "EndGesture" ? [s.command.command.gesture] : []));
  return { edits: new Set(edits.map((s) => s.gesture)), ends };
}

let transports: EngineTransport[] = [];
const mock = () => {
  const m = new MockTransport({ timers: "manual", seed: 1 });
  transports.push(m);
  return m;
};

beforeEach(() => resetArrangementUi());
afterEach(() => {
  for (const t of transports) t.dispose();
  transports = [];
  store().reset();
});

describe("dropBrowserMedia", () => {
  it("imports and creates the clip as one undo step", async () => {
    const t = mock();
    const sent = recording(t);
    await connect(t);
    const before = counts();
    const id = await dropBrowserMedia(t, kick, { track: drums(), at: 32 });
    await flush();
    expect(project().clips[id]).toMatchObject({ track: drums(), start: 32, name: "Kick" });
    expect(counts()).toEqual({ ...before, clips: before.clips + 1, media: before.media + 1 });
    const { edits, ends } = gestures(sent);
    expect(edits.size).toBe(1);
    expect(ends).toEqual([...edits]);
    expect(useArrangementUi.getState().imports).toEqual([]);

    await undo(t);
    expect(counts()).toEqual(before);
  });

  it("creates a new track with the clip in the same step", async () => {
    const t = mock();
    await connect(t);
    const before = counts();
    const id = await dropBrowserMedia(t, kick, { track: null, at: 0 });
    expect(project().tracks[project().clips[id]!.track]?.kind).toBe("Audio");
    expect(counts()).toEqual({ tracks: before.tracks + 1, clips: before.clips + 1, media: before.media + 1 });
    await undo(t);
    expect(counts()).toEqual(before);
  });

  it("waits for the length of media imported without one, showing progress", async () => {
    const t = new VbrTransport(mock());
    await connect(t);
    const before = counts();
    let done: string | null = null;
    const drop = dropBrowserMedia(t, kick, { track: drums(), at: 36 }).then((id) => (done = id));
    await flush();

    const imported = t.sent.find((s) => s.command.domain === "Media")!.command;
    const media = (imported.command as { id: MediaId }).id;
    expect(project().media[media]?.frames).toBe(0);
    expect(t.sent.some((s) => createAudioMedia(s.command).length > 0)).toBe(false);
    expect(useArrangementUi.getState().imports).toMatchObject([{ track: drums(), at: 36, name: "Kick.wav", progress: null, error: null }]);

    t.emit({ type: "Media", event: { type: "ImportProgress", media, progress: 0.5 } });
    expect(useArrangementUi.getState().imports[0]?.progress).toBe(0.5);
    expect(done).toBeNull();

    expect(t.decode(media).frames).toBeGreaterThan(0);
    await drop;
    expect(project().media[media]?.frames).toBeGreaterThan(0);
    const clip = project().clips[done!]!;
    expect(clip).toMatchObject({ track: drums(), start: 36, name: "Kick" });
    expect(clip.length).toBeGreaterThan(0);
    expect(useArrangementUi.getState().imports).toEqual([]);
    const { edits, ends } = gestures(t.sent);
    expect(edits.size).toBe(1);
    expect(ends).toEqual([...edits]);

    // Import + clip are still one undo step.
    await undo(t);
    expect(counts()).toEqual(before);
  });

  it("fails with a visible error when the length never arrives", async () => {
    const t = new VbrTransport(mock());
    await connect(t);
    await expect(dropBrowserMedia(t, kick, { track: drums(), at: 36 }, { timeoutMs: 10 })).rejects.toThrow(/timed out/);
    expect(useArrangementUi.getState().imports).toMatchObject([{ name: "Kick.wav", error: expect.stringMatching(/timed out/) }]);
    expect(t.sent.some((s) => createAudioMedia(s.command).length > 0)).toBe(false);
    expect(t.sent.at(-1)?.command).toMatchObject({ domain: "Edit", command: { type: "EndGesture" } });
  });

  it("fails when the engine reports the media missing", async () => {
    const t = new VbrTransport(mock());
    await connect(t);
    const drop = dropBrowserMedia(t, kick, { track: drums(), at: 36 });
    await flush();
    const media = (t.sent.find((s) => s.command.domain === "Media")!.command.command as { id: MediaId }).id;
    t.emit({ type: "Media", event: { type: "Missing", media } });
    await expect(drop).rejects.toThrow(/missing/);
  });
});
