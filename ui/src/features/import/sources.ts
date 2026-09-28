/**
 * What the user imports (CONTRACTS.md §12.13): a file of the UI machine the browser can
 * read (a `File` from a drop or the web file picker: its bytes are uploaded) or a path of
 * the engine machine (the desktop file dialog / an OS drop: the engine reads it; the UI
 * never does).
 */
import type { UploadSource } from "@/features/remote/upload";

export type ImportSource = { kind: "file"; file: UploadSource } | { kind: "path"; path: string };

/** Audio extensions the engine decodes (`ether_controller::store::file_kind`). */
export const AUDIO_EXTENSIONS = ["wav", "wave", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga"] as const;

/** `accept` of the web file picker. */
export const AUDIO_ACCEPT = ["audio/*", ...AUDIO_EXTENSIONS.map((e) => `.${e}`)].join(",");

/** Largest file imported (the engine's upload / path limit: 1 GiB). */
export const MAX_IMPORT_BYTES = 1024 * 1024 * 1024;

export const fileSource = (file: UploadSource): ImportSource => ({ kind: "file", file });
export const pathSource = (path: string): ImportSource => ({ kind: "path", path });

/** Display name: the file name (either path separator). */
export function sourceName(s: ImportSource): string {
  if (s.kind === "file") return s.file.name;
  return s.path.split(/[\\/]/).pop() || s.path;
}

/** Bytes to upload (`null` for a path: the engine reads it). */
export function sourceSize(s: ImportSource): number | null {
  return s.kind === "file" ? s.file.size : null;
}

function extension(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(dot + 1).toLowerCase();
}

export function isAudioName(name: string): boolean {
  return (AUDIO_EXTENSIONS as readonly string[]).includes(extension(name));
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024 * 1024) return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1024))} KB`;
}

/**
 * Why `s` can't be imported, checked before anything is sent (the engine checks again):
 * not an audio file by extension, empty, or over the size limit. `null` = fine.
 */
export function rejectReason(s: ImportSource): string | null {
  const name = sourceName(s);
  if (!isAudioName(name)) {
    const ext = extension(name);
    return ext ? `.${ext} files are not supported (use WAV, AIFF, FLAC, MP3 or OGG)` : "not an audio file";
  }
  const size = sourceSize(s);
  if (size !== null && size <= 0) return "the file is empty";
  if (size !== null && size > MAX_IMPORT_BYTES) return `too large (${formatBytes(size)}; the limit is ${formatBytes(MAX_IMPORT_BYTES)})`;
  return null;
}
