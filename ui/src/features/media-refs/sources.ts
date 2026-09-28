/**
 * Where a relink's file comes from. The desktop passes paths from the OS dialogs (the engine
 * reads them: `MediaSource::Path`); the web and remote UIs upload the bytes through the
 * frozen staging (`BeginUpload` → `UploadChunk`s) and relink to the upload (copied into the
 * project). The UI never reads files on the desktop.
 */

import type { MediaRef, MediaSource } from "@/generated";
import { isPathDropHost, pickFiles } from "@/features/import";
import { cmd, newId, type EngineTransport } from "@/transport";
import { bytesToBase64 } from "@/transport/ws/binaryFrame";

/** The desktop transport's folder dialog (`TauriTransport.pickFolder`). */
export interface FolderPickerHost {
  pickFolder(): Promise<string | null>;
}

export function hasFolderPicker(t: EngineTransport | null | undefined): t is EngineTransport & FolderPickerHost {
  return typeof (t as Partial<FolderPickerHost> | null | undefined)?.pickFolder === "function";
}

/** Bytes per staged chunk (the protocol allows up to 1 MiB). */
const CHUNK_BYTES = 256 * 1024;

/** What `stageUpload` needs from a `File` (tests pass plain objects). */
export interface StageFile {
  name: string;
  size: number;
  slice(start: number, end: number): { arrayBuffer(): Promise<ArrayBuffer> };
}

/** Upload `file` to the engine's staging; resolves with its `Upload` source. */
export async function stageUpload(transport: EngineTransport, file: StageFile): Promise<MediaSource> {
  if (file.size <= 0) throw new Error(`${file.name} is empty`);
  const upload = newId();
  await transport.send(cmd("Media", { type: "BeginUpload", upload, name: file.name, size: file.size }));
  try {
    for (let offset = 0; offset < file.size; offset += CHUNK_BYTES) {
      const bytes = new Uint8Array(await file.slice(offset, Math.min(file.size, offset + CHUNK_BYTES)).arrayBuffer());
      await transport.send(cmd("Media", { type: "UploadChunk", upload, offset, data: bytesToBase64(bytes) }));
    }
  } catch (e) {
    transport.send(cmd("Media", { type: "CancelUpload", upload })).catch(() => {});
    throw e;
  }
  return { type: "Upload", upload };
}

/**
 * Ask the user for the file to relink to: the OS dialog on the desktop (a path), the
 * browser's file picker elsewhere (staged upload). `null` when dismissed.
 */
export async function pickRelinkSource(transport: EngineTransport): Promise<MediaSource | null> {
  if (isPathDropHost(transport)) {
    const paths = await transport.pickAudioFiles();
    const path = paths?.[0];
    return path ? { type: "Path", path } : null;
  }
  const [file] = await pickFiles();
  return file ? stageUpload(transport, file) : null;
}

/** Display text of a candidate or source. */
export function sourceLabel(source: MediaSource): string {
  switch (source.type) {
    case "Path":
      return source.path;
    case "Location":
      return source.location.type === "Library" ? `Library/${source.path}` : `Project/${source.path}`;
    case "Upload":
      return "Uploaded file";
    case "Project":
      return "Project media";
  }
}

/** Where a media is read from, for display. */
export function locationLabel(m: MediaRef): string {
  return m.location.type === "External" ? m.location.path : "Project folder";
}
