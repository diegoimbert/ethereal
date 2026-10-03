/**
 * Mock of uploads from the UI machine (`Media::{BeginUpload, UploadChunk, CancelUpload}`,
 * `MediaSource::Upload`). Owned by `remote-engine`; `file-import` made the mock stage them,
 * like every real host now does (native disk, web OPFS), so "Import audio…" and OS drops
 * work against the mock too.
 *
 * Rules (as `crates/ether-controller/src/upload/mod.rs`, which is the spec): size
 * `1..=1 GiB`, a display name, in-order chunks (a retried chunk is a no-op, a gap is
 * `InvalidState`), `UploadProgress` events, `CancelUpload` drops it. Importing a completed
 * upload reads the WAV header for rate/channels/length; other audio extensions import as
 * 2 s of 44.1 kHz stereo; anything else fails with `Decode`.
 */

import type { Event, MediaCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

export type UploadCommand = Extract<MediaCommand, { type: "BeginUpload" | "UploadChunk" | "CancelUpload" }>;

export const MAX_UPLOAD_BYTES = 1024 * 1024 * 1024;
const MAX_CHUNK_BYTES = 1024 * 1024;
const AUDIO = /\.(wav|wave|aif|aiff|aifc|flac|mp3|ogg|oga)$/i;

interface Staged {
  name: string;
  size: number;
  chunks: Uint8Array[];
  received: number;
}

/** What an import of a completed upload creates. */
export interface UploadedAudio {
  name: string;
  sample_rate: number;
  channels: number;
  frames: number;
  /** Fake content hash (dedupes identical uploads). */
  hash: string;
}

function base64ToBytes(b64: string): Uint8Array {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

function fnv(bytes: Uint8Array): string {
  let h = 2166136261;
  for (const b of bytes) h = Math.imul(h ^ b, 16777619);
  return (h >>> 0).toString(16).padStart(8, "0");
}

/** Rate, channels and frames of a RIFF/WAVE file, or `null`. */
export function wavInfo(bytes: Uint8Array): { sample_rate: number; channels: number; frames: number } | null {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const tag = (at: number) => String.fromCharCode(...bytes.subarray(at, at + 4));
  if (bytes.length < 12 || tag(0) !== "RIFF" || tag(8) !== "WAVE") return null;
  let fmt: { channels: number; rate: number; align: number } | null = null;
  for (let at = 12; at + 8 <= bytes.length; ) {
    const id = tag(at);
    const len = v.getUint32(at + 4, true);
    if (id === "fmt " && at + 24 <= bytes.length) {
      fmt = { channels: v.getUint16(at + 10, true), rate: v.getUint32(at + 12, true), align: v.getUint16(at + 20, true) };
    } else if (id === "data" && fmt && fmt.align > 0 && fmt.channels > 0 && fmt.rate > 0) {
      const data = Math.min(len, bytes.length - at - 8);
      return { sample_rate: fmt.rate, channels: fmt.channels, frames: Math.floor(data / fmt.align) };
    }
    at += 8 + len + (len % 2);
  }
  return null;
}

export class MockUploads {
  private readonly uploads = new Map<string, Staged>();

  constructor(private readonly emit: (event: Event) => void) {}

  command(c: UploadCommand): ReplyValue {
    switch (c.type) {
      case "BeginUpload": {
        if (!Number.isFinite(c.size) || c.size < 1 || c.size > MAX_UPLOAD_BYTES) fail("InvalidArgument", `upload size must be 1..=${MAX_UPLOAD_BYTES} bytes`);
        const name = c.name.split(/[\\/]/).pop()!.trim();
        if (!name) fail("InvalidArgument", "upload name is empty");
        this.uploads.set(c.upload, { name, size: c.size, chunks: [], received: 0 });
        return { type: "Unit" };
      }
      case "UploadChunk": {
        const u = this.uploads.get(c.upload) ?? fail("NotFound", `upload ${c.upload}`);
        const bytes = base64ToBytes(c.data);
        if (bytes.length > MAX_CHUNK_BYTES) fail("InvalidArgument", `upload chunks are at most ${MAX_CHUNK_BYTES} bytes`);
        const end = c.offset + bytes.length;
        if (c.offset < u.received && end <= u.received) return { type: "Unit" };
        if (c.offset !== u.received) fail("InvalidState", `upload ${c.upload}: expected offset ${u.received}, got ${c.offset}`);
        if (end > u.size) fail("InvalidArgument", `upload ${c.upload}: ${end} bytes exceed the announced ${u.size}`);
        u.chunks.push(bytes);
        u.received = end;
        this.emit({ type: "Media", event: { type: "UploadProgress", upload: c.upload, received: u.received } });
        return { type: "Unit" };
      }
      case "CancelUpload":
        this.uploads.delete(c.upload);
        return { type: "Unit" };
    }
  }

  /** Validate `MediaSource::Upload` (preview): the upload exists and is complete. */
  check(upload: string): Staged {
    const u = this.uploads.get(upload) ?? fail("NotFound", `upload ${upload}`);
    if (u.received !== u.size) fail("InvalidState", `upload ${upload} is incomplete (${u.received} of ${u.size} bytes)`);
    return u;
  }

  /** Consume a completed upload's raw bytes (base-114: `Project::ImportBundle`). */
  takeBytes(upload: string): Uint8Array {
    const u = this.check(upload);
    this.uploads.delete(upload);
    const bytes = new Uint8Array(u.size);
    let at = 0;
    for (const c of u.chunks) {
      bytes.set(c, at);
      at += c.length;
    }
    return bytes;
  }

  /** Consume a completed upload for `Import { source: Upload }` (see the module docs). */
  take(upload: string): UploadedAudio {
    const u = this.check(upload);
    this.uploads.delete(upload);
    const bytes = new Uint8Array(u.size);
    let at = 0;
    for (const c of u.chunks) {
      bytes.set(c, at);
      at += c.length;
    }
    if (!AUDIO.test(u.name)) fail("Decode", `${u.name} is not an audio file`);
    const wav = /\.wave?$/i.test(u.name) ? wavInfo(bytes) : null;
    if (/\.wave?$/i.test(u.name) && !wav) fail("Decode", `${u.name}: no decodable audio`);
    return { name: u.name, hash: fnv(bytes), ...(wav ?? { sample_rate: 44100, channels: 2, frames: 2 * 44100 }) };
  }
}
