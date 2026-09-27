/**
 * Uploading files from the UI machine to a remote engine (`Media::BeginUpload` →
 * `UploadChunk`s → `Import { source: Upload }`; see `ether_protocol::media`).
 *
 * Chunks go out as binary frames when the transport can (`WsTransport.uploadChunk`), else
 * as base64 JSON. A few chunks are kept in flight (the server applies them in order); any
 * failure cancels the upload. Progress is published to `uploadStore` for the top bar.
 */
import { useSyncExternalStore } from "react";
import type { MediaRef, ReplyValue } from "@/generated";
import { cmd, newId, type EngineTransport } from "@/transport";
import { bytesToBase64 } from "@/transport/ws/binaryFrame";

/** Bytes per chunk (the protocol allows up to 1 MiB). */
export const CHUNK_BYTES = 256 * 1024;
/** Chunks sent before waiting for the oldest reply. */
export const IN_FLIGHT = 4;

interface ChunkSender {
  uploadChunk(upload: string, offset: number, bytes: Uint8Array): Promise<ReplyValue>;
}

function hasBinaryChunks(t: EngineTransport): t is EngineTransport & ChunkSender {
  return typeof (t as Partial<ChunkSender>).uploadChunk === "function";
}

/** What `uploadFile` needs from a `File` (tests pass plain objects). */
export interface UploadSource {
  name: string;
  size: number;
  slice(start: number, end: number): { arrayBuffer(): Promise<ArrayBuffer> };
}

export interface UploadProgress {
  id: string;
  name: string;
  size: number;
  sent: number;
  error: string | null;
  done: boolean;
}

/** Upload `file` and import it into the open project. Resolves with the new media. */
export async function uploadFile(transport: EngineTransport, file: UploadSource, onProgress?: (sent: number) => void): Promise<MediaRef> {
  if (file.size <= 0) throw new Error(`${file.name} is empty`);
  const upload = newId();
  await transport.send(cmd("Media", { type: "BeginUpload", upload, name: file.name, size: file.size }));
  try {
    const inFlight: Promise<unknown>[] = [];
    let sent = 0;
    for (let offset = 0; offset < file.size; offset += CHUNK_BYTES) {
      const end = Math.min(file.size, offset + CHUNK_BYTES);
      const bytes = new Uint8Array(await file.slice(offset, end).arrayBuffer());
      const reply = hasBinaryChunks(transport)
        ? transport.uploadChunk(upload, offset, bytes)
        : transport.send(cmd("Media", { type: "UploadChunk", upload, offset, data: bytesToBase64(bytes) }));
      const done = reply.then(() => {
        sent += bytes.length;
        onProgress?.(sent);
      });
      done.catch(() => {}); // awaited below; don't report it twice
      inFlight.push(done);
      if (inFlight.length >= IN_FLIGHT) await inFlight.shift();
    }
    await Promise.all(inFlight);
    const reply = await transport.send(cmd("Media", { type: "Import", id: newId(), source: { type: "Upload", upload } }));
    if (reply.type !== "Media") throw new Error(`unexpected reply ${reply.type}`);
    return reply.media;
  } catch (e) {
    transport.send(cmd("Media", { type: "CancelUpload", upload })).catch(() => {});
    throw e;
  }
}

// ---- Progress store (top bar status) ---------------------------------------------------

type Listener = () => void;
let uploads: readonly UploadProgress[] = [];
const listeners = new Set<Listener>();

function set(next: readonly UploadProgress[]) {
  uploads = next;
  for (const l of [...listeners]) l();
}

export const uploadStore = {
  get: () => uploads,
  subscribe(l: Listener) {
    listeners.add(l);
    return () => void listeners.delete(l);
  },
  start(p: UploadProgress) {
    set([...uploads.filter((u) => u.id !== p.id), p]);
  },
  update(id: string, patch: Partial<UploadProgress>) {
    set(uploads.map((u) => (u.id === id ? { ...u, ...patch } : u)));
  },
  remove(id: string) {
    set(uploads.filter((u) => u.id !== id));
  },
  clear() {
    set([]);
  },
};

export function useUploads(): readonly UploadProgress[] {
  return useSyncExternalStore(uploadStore.subscribe, uploadStore.get);
}

/** Upload several files one after the other, tracking them in `uploadStore`. */
export async function uploadFiles(transport: EngineTransport, files: readonly UploadSource[]): Promise<MediaRef[]> {
  const out: MediaRef[] = [];
  for (const file of files) {
    const id = newId();
    uploadStore.start({ id, name: file.name, size: file.size, sent: 0, error: null, done: false });
    try {
      out.push(await uploadFile(transport, file, (sent) => uploadStore.update(id, { sent })));
      uploadStore.remove(id);
    } catch (e) {
      uploadStore.update(id, { error: e instanceof Error ? e.message : String(e), done: true });
    }
  }
  return out;
}
