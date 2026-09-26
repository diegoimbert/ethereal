/**
 * Drag payload of the sample browser: the contract between the browser (drag source) and
 * drop targets such as the arrangement or session views.
 *
 * ## Format
 * - MIME type: `BROWSER_DRAG_MIME` (`application/x-ethereal-media+json`).
 * - Data: JSON of `BrowserDragPayload` (below). `text/plain` also carries the file name,
 *   for drops outside the app.
 * - `effectAllowed = "copy"`.
 *
 * The payload never contains a file-system path: `source` is a `MediaSource` the engine
 * resolves (a relative path inside an engine-visible browse location, or media already in
 * the project).
 *
 * ## Using it in a drop target
 * ```ts
 * onDragOver = (e) => { if (hasBrowserDrag(e.dataTransfer)) { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; } };
 * onDrop = async (e) => {
 *   const payload = readBrowserDrag(e.dataTransfer);
 *   if (!payload) return;
 *   e.preventDefault();
 *   const media = await resolveDroppedMedia(transport, payload); // imports if needed
 *   await transport.send(cmd("Clip", {
 *     type: "CreateAudio", id: newId(), track, location: { type: "Arrangement", start }, media: media.id,
 *   }));
 * };
 * ```
 * During `dragover` browsers only expose the MIME types, not the data: use
 * `hasBrowserDrag`, then read the payload on `drop`.
 */
import type { FileKind, MediaRef, MediaSource } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";

export const BROWSER_DRAG_MIME = "application/x-ethereal-media+json";

export interface BrowserDragPayload {
  /** Payload format version (bumped on incompatible changes). */
  version: 1;
  kind: "media";
  /** Where the engine reads the file from. Pass to `Media::Import` (see `resolveDroppedMedia`). */
  source: MediaSource;
  /** Display name (file name). */
  name: string;
  /** Always `"Audio"` in v0.1 (only audio files are draggable). */
  file_kind: FileKind;
}

/** Minimal DataTransfer surface used here (easy to fake in tests). */
export interface DragData {
  types: ReadonlyArray<string>;
  getData(format: string): string;
  setData?(format: string, data: string): void;
  effectAllowed?: DataTransfer["effectAllowed"];
}

/** Put `payload` on a drag's `DataTransfer` (call from `onDragStart`). */
export function writeBrowserDrag(dt: DataTransfer, payload: BrowserDragPayload): void {
  dt.setData(BROWSER_DRAG_MIME, JSON.stringify(payload));
  dt.setData("text/plain", payload.name);
  dt.effectAllowed = "copy";
}

/** `true` if the drag carries a browser payload (usable during `dragover`). */
export function hasBrowserDrag(dt: Pick<DragData, "types"> | null | undefined): boolean {
  return !!dt && Array.from(dt.types).includes(BROWSER_DRAG_MIME);
}

/** The payload of a drop, or `null` if absent/malformed. */
export function readBrowserDrag(dt: Pick<DragData, "getData" | "types"> | null | undefined): BrowserDragPayload | null {
  if (!hasBrowserDrag(dt)) return null;
  try {
    return parseBrowserDragPayload(JSON.parse(dt!.getData(BROWSER_DRAG_MIME)));
  } catch {
    return null;
  }
}

/** Validate an unknown value as a `BrowserDragPayload`. */
export function parseBrowserDragPayload(value: unknown): BrowserDragPayload | null {
  if (typeof value !== "object" || value === null) return null;
  const v = value as Partial<BrowserDragPayload>;
  if (v.version !== 1 || v.kind !== "media" || typeof v.name !== "string" || typeof v.file_kind !== "string") return null;
  const s = v.source as Partial<MediaSource> | undefined;
  if (!s || typeof s !== "object") return null;
  const ok =
    (s.type === "Location" && typeof s.path === "string" && typeof s.location === "object" && s.location !== null) ||
    (s.type === "Project" && typeof s.media === "string");
  return ok ? (v as BrowserDragPayload) : null;
}

/**
 * The project `MediaRef` for a dropped payload: media already in the project is returned
 * as is; anything else is imported first (`Media::Import`, one undo step). Rejects with
 * `CommandFailedError` if the engine can't import it.
 */
export async function resolveDroppedMedia(transport: EngineTransport, payload: BrowserDragPayload): Promise<MediaRef> {
  const { source } = payload;
  if (source.type === "Project") {
    const existing = useProjectStore.getState().project?.media[source.media];
    if (existing) return existing;
  }
  const reply = await transport.send(cmd("Media", { type: "Import", id: newId(), source }));
  if (reply.type !== "Media") throw new Error(`unexpected reply to Media::Import: ${reply.type}`);
  return reply.media;
}
