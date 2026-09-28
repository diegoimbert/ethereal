import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, GestureId } from "@/generated";
import { resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { useProjectStore } from "@/state";
import { cmd, MockTransport, type EngineTransport } from "@/transport";
import { commandTarget, importAudio } from "./importAudio";
import { useImportStore } from "./importStore";
import { deliverPathDrop, hasOsFiles, noteHover, resetHover } from "./osDrop";
import { fileSource, MAX_IMPORT_BYTES, pathSource, rejectReason, sourceName, type ImportSource } from "./sources";

/** A 16-bit PCM WAV of `frames` frames at 48 kHz mono. */
function wav(frames: number): Uint8Array {
  const data = frames * 2;
  const b = new Uint8Array(44 + data);
  const v = new DataView(b.buffer);
  const tag = (at: number, s: string) => [...s].forEach((c, i) => (b[at + i] = c.charCodeAt(0)));
  tag(0, "RIFF");
  v.setUint32(4, 36 + data, true);
  tag(8, "WAVE");
  tag(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, 48000, true);
  v.setUint32(28, 96000, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  tag(36, "data");
  v.setUint32(40, data, true);
  return b;
}

const file = (name: string, bytes: Uint8Array = wav(48000)) => fileSource(new File([bytes.slice().buffer], name));

const store = () => useProjectStore.getState();
const project = () => store().project!;
const audioTracks = () => Object.values(project().tracks).filter((t) => t.kind === "Audio");

type Sent = Array<{ command: Command; gesture: GestureId | undefined }>;

let transports: EngineTransport[] = [];
async function connected(): Promise<{ t: MockTransport; sent: Sent }> {
  const t = new MockTransport({ timers: "manual", seed: 3 });
  transports.push(t);
  const sent: Sent = [];
  const send = t.send.bind(t);
  t.send = (command, opts) => {
    sent.push({ command, gesture: opts?.gesture });
    return send(command, opts);
  };
  t.onEvent((e) => {
    if (e.type === "Patch") store().applyPatch(e.patch);
  });
  store().loadProject(await t.connect());
  return { t, sent };
}

beforeEach(() => {
  resetArrangementUi();
  resetHover();
  useImportStore.setState({ items: [] });
});
afterEach(() => {
  for (const t of transports) t.dispose();
  transports = [];
  store().reset();
});

describe("sources", () => {
  it("names and rejects before sending anything", () => {
    expect(sourceName(pathSource("/Users/me/Kick 01.wav"))).toBe("Kick 01.wav");
    expect(sourceName(pathSource("C:\\Samples\\Snare.aiff"))).toBe("Snare.aiff");
    expect(rejectReason(file("a.WAV"))).toBeNull();
    expect(rejectReason(pathSource("/x/loop.flac"))).toBeNull();
    expect(rejectReason(file("notes.txt"))).toMatch(/\.txt files are not supported/);
    expect(rejectReason(file("a.wav", new Uint8Array()))).toBe("the file is empty");
    const huge: ImportSource = { kind: "file", file: { name: "big.wav", size: MAX_IMPORT_BYTES + 1, slice: () => new Blob() } };
    expect(rejectReason(huge)).toMatch(/too large/);
  });

  it("recognizes OS file drags", () => {
    expect(hasOsFiles({ types: ["Files"] })).toBe(true);
    expect(hasOsFiles({ types: ["application/x-ethereal-media+json"] })).toBe(false);
  });
});

describe("importAudio", () => {
  it("uploads files onto a track, one after the other, as one undo step", async () => {
    const { t, sent } = await connected();
    const track = audioTracks()[0]!;
    const before = Object.keys(project().clips).length;
    const out = await importAudio(t, [file("A.wav"), file("B.wav")], { track: track.id, at: 4 });
    expect(out.map((o) => o.error)).toEqual([null, null]);
    const [a, b] = out.map((o) => project().clips[o.clip!]!);
    expect(a!.track).toBe(track.id);
    expect(a!.start).toBe(4);
    expect(b!.start).toBeCloseTo(a!.start + a!.length);
    expect(project().media[out[0]!.media!.id]!.name).toBe("A.wav");
    // One gesture for every edit, closed at the end.
    const edits = sent.filter((s) => !["BeginUpload", "UploadChunk", "EndGesture"].includes(s.command.command.type));
    expect(new Set(edits.map((s) => s.gesture)).size).toBe(1);
    expect(sent.at(-1)!.command).toMatchObject({ domain: "Edit", command: { type: "EndGesture" } });
    await t.send(cmd("Edit", { type: "Undo" }));
    expect(Object.keys(project().clips).length).toBe(before);
    expect(useImportStore.getState().items).toEqual([]);
    expect(useArrangementUi.getState().imports).toEqual([]);
  });

  it("puts each file on a new audio track below the tracks", async () => {
    const { t } = await connected();
    const n = audioTracks().length;
    const out = await importAudio(t, [file("Pad.wav"), file("Bass.wav")], { track: null, at: 8 });
    expect(audioTracks().length).toBe(n + 2);
    const clips = out.map((o) => project().clips[o.clip!]!);
    expect(clips.map((c) => c.start)).toEqual([8, 8]);
    expect(clips[0]!.track).not.toBe(clips[1]!.track);
  });

  it("keeps going after a bad file and reports it until dismissed", async () => {
    const { t } = await connected();
    const out = await importAudio(t, [file("readme.txt"), file("broken.wav", new Uint8Array([1, 2, 3])), file("ok.wav")]);
    expect(out.map((o) => o.error)).toEqual([expect.stringMatching(/not supported/), "not a readable audio file", null]);
    const rows = useImportStore.getState().items;
    expect(rows.map((r) => r.error)).toEqual([expect.stringMatching(/^readme.txt: /), "broken.wav: not a readable audio file"]);
    expect(out[2]!.media).not.toBeNull();
    expect(out[2]!.clip).toBeNull();
  });

  it("cancels an upload", async () => {
    const { t, sent } = await connected();
    const media = Object.keys(project().media).length;
    const done = importAudio(t, [file("Long.wav", wav(48000 * 8))], { track: null, at: 0 });
    // Cancel as soon as the row exists.
    useImportStore.getState().items[0]!.cancel!();
    const [o] = await done;
    expect(o!.error).toBe("cancelled");
    expect(useImportStore.getState().items).toEqual([]);
    expect(Object.keys(project().media)).toHaveLength(media);
    expect(sent.some((s) => s.command.command.type === "Import")).toBe(false);
  });

  it("path imports reach the engine as paths (the mock has no OS files)", async () => {
    const { t, sent } = await connected();
    const [o] = await importAudio(t, [pathSource("/Users/me/Kick.wav")]);
    expect(sent.find((s) => s.command.command.type === "Import")!.command).toMatchObject({
      domain: "Media",
      command: { type: "Import", source: { type: "Path", path: "/Users/me/Kick.wav" } },
    });
    expect(o!.error).toMatch(/desktop engine/);
  });

  it("targets the selected audio track, else new tracks", async () => {
    await connected();
    const audio = audioTracks()[0]!;
    const midi = Object.values(project().tracks).find((t) => t.kind === "Midi")!;
    expect(commandTarget(audio.id, 3)).toEqual({ track: audio.id, at: 3 });
    expect(commandTarget(midi.id, 3)).toEqual({ track: null, at: 3 });
    expect(commandTarget(null, -1)).toEqual({ track: null, at: 0 });
  });
});

describe("path drops", () => {
  it("land in the zone hovered last, at its point", () => {
    const got: Array<{ sources: ImportSource[]; x: number }> = [];
    expect(deliverPathDrop(["/a.wav"])).toBe(false);
    noteHover({ clientX: 10, clientY: 20, altKey: false }, (sources, p) => got.push({ sources, x: p.clientX }), 1000);
    expect(deliverPathDrop(["/a.wav", "/b.wav"], 1500)).toBe(true);
    expect(got).toEqual([{ sources: [pathSource("/a.wav"), pathSource("/b.wav")], x: 10 }]);
    // Consumed; stale hovers are ignored.
    expect(deliverPathDrop(["/a.wav"], 1600)).toBe(false);
    noteHover({ clientX: 0, clientY: 0, altKey: false }, () => got.push({ sources: [], x: -1 }), 0);
    expect(deliverPathDrop(["/a.wav"], 10_000)).toBe(false);
  });
});
