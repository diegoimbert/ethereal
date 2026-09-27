import { afterEach, describe, expect, it } from "vitest";
import type { Command, MediaRef, ReplyValue } from "@/generated";
import { CommandFailedError, Emitter, type EngineTransport } from "@/transport";
import { CHUNK_BYTES, uploadFile, uploadFiles, uploadStore } from "./upload";

const MEDIA: MediaRef = { id: "m", name: "a.wav", file: "media/a.wav", sample_rate: 48000, channels: 1, frames: 10, hash: null, location: { type: "Project" } };

function file(size: number, name = "a.wav") {
  const bytes = new Uint8Array(size).map((_, i) => i % 251);
  return {
    name,
    size,
    slice: (a: number, b: number) => ({ arrayBuffer: () => Promise.resolve(bytes.slice(a, b).buffer) }),
    bytes,
  };
}

/** Records every command; `failAt` rejects the chunk at that offset. */
class FakeTransport implements EngineTransport {
  readonly kind = "remote" as const;
  readonly log: { type: string; offset?: number; size?: number; binary?: boolean }[] = [];
  private parts: Uint8Array[] = [];
  get received(): number[] {
    return this.parts.flatMap((p) => Array.from(p));
  }
  failAt: number | null = null;
  constructor(binary: boolean) {
    if (!binary) (this as { uploadChunk?: unknown }).uploadChunk = undefined;
  }
  connect(): never {
    throw new Error("unused");
  }
  send(command: Command): Promise<ReplyValue> {
    if (command.domain !== "Media") throw new Error("unexpected");
    const c = command.command;
    if (c.type === "UploadChunk") {
      const bytes = Uint8Array.from(atob(c.data), (ch) => ch.charCodeAt(0));
      return this.chunk(c.offset, bytes, false);
    }
    this.log.push({ type: c.type, ...(c.type === "BeginUpload" ? { size: c.size } : {}) });
    if (c.type === "Import") return Promise.resolve({ type: "Media", media: MEDIA });
    return Promise.resolve({ type: "Unit" });
  }
  uploadChunk(_upload: string, offset: number, bytes: Uint8Array): Promise<ReplyValue> {
    return this.chunk(offset, bytes, true);
  }
  private chunk(offset: number, bytes: Uint8Array, binary: boolean): Promise<ReplyValue> {
    this.log.push({ type: "UploadChunk", offset, binary });
    if (offset === this.failAt) return Promise.reject(new CommandFailedError({ code: "Io", message: "disk full" }));
    this.parts.push(bytes.slice());
    return Promise.resolve({ type: "Unit" });
  }
  onEvent = (l: never) => new Emitter().on(l);
  subscribePlayhead = (l: never) => new Emitter().on(l);
  subscribeMeters = (l: never) => new Emitter().on(l);
  dispose() {}
}

afterEach(() => uploadStore.clear());

describe("uploadFile", () => {
  it("begins, sends ordered binary chunks, imports", async () => {
    const t = new FakeTransport(true);
    const f = file(CHUNK_BYTES * 2 + 10);
    const progress: number[] = [];
    await expect(uploadFile(t, f, (s) => progress.push(s))).resolves.toEqual(MEDIA);
    expect(t.log.map((l) => l.type)).toEqual(["BeginUpload", "UploadChunk", "UploadChunk", "UploadChunk", "Import"]);
    expect(t.log[0]!.size).toBe(f.size);
    expect(t.log.filter((l) => l.type === "UploadChunk").map((l) => [l.offset, l.binary])).toEqual([
      [0, true],
      [CHUNK_BYTES, true],
      [CHUNK_BYTES * 2, true],
    ]);
    expect(t.received).toEqual([...f.bytes]);
    expect(progress.at(-1)).toBe(f.size);
  });

  it("falls back to base64 JSON chunks", async () => {
    const t = new FakeTransport(false);
    const f = file(1000);
    await uploadFile(t, f);
    expect(t.log.find((l) => l.type === "UploadChunk")?.binary).toBe(false);
    expect(t.received).toEqual([...f.bytes]);
  });

  it("cancels the upload when a chunk fails", async () => {
    const t = new FakeTransport(true);
    t.failAt = CHUNK_BYTES;
    await expect(uploadFile(t, file(CHUNK_BYTES * 3))).rejects.toThrow(/disk full/);
    expect(t.log.at(-1)?.type).toBe("CancelUpload");
    expect(t.log.some((l) => l.type === "Import")).toBe(false);
  });

  it("refuses empty files", async () => {
    await expect(uploadFile(new FakeTransport(true), file(0))).rejects.toThrow(/empty/);
  });
});

describe("uploadFiles", () => {
  it("tracks failures in the store and drops finished uploads", async () => {
    const t = new FakeTransport(true);
    const out = await uploadFiles(t, [file(10, "ok.wav"), file(0, "empty.wav")]);
    expect(out).toHaveLength(1);
    const left = uploadStore.get();
    expect(left).toHaveLength(1);
    expect(left[0]).toMatchObject({ name: "empty.wav", done: true });
    expect(left[0]!.error).toMatch(/empty/);
  });
});
