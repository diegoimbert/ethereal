/**
 * Drag payload of the sample browser: the contract between the browser (drag source) and
 * drop targets such as the arrangement view.
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
 *   const gesture = nextGestureId(); // import + clip = one undo step
 *   try {
 *     const media = await resolveDroppedMedia(transport, payload, { gesture }); // imports if needed
 *     await waitForMediaLength(transport, media.id); // VBR media: length known after decode
 *     await transport.send(cmd("Clip", { type: "CreateAudio", id: newId(), track, start, media: media.id }), { gesture });
 *   } finally {
 *     void transport.send(cmd("Edit", { type: "EndGesture", gesture }));
 *   }
 * };
 * ```
 * During `dragover` browsers only expose the MIME types, not the data: use
 * `hasBrowserDrag`, then read the payload on `drop`.
 */
import type { FileKind, GestureId, MediaId, MediaRef, MediaSource } from "@/generated";
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
 * as is; anything else is imported first (`Media::Import`; its own undo step unless
 * `opts.gesture` merges it with the commands that follow). Rejects with
 * `CommandFailedError` if the engine can't import it.
 *
 * The returned media may have `frames: 0` (length unknown until the background decode,
 * e.g. VBR MP3): `Clip::CreateAudio` fails with `InvalidState` until then. Use
 * `waitForMediaLength` before creating clips from it.
 */
export async function resolveDroppedMedia(
  transport: EngineTransport,
  payload: BrowserDragPayload,
  opts: { gesture?: GestureId } = {},
): Promise<MediaRef> {
  const { source } = payload;
  if (source.type === "Project") {
    const existing = useProjectStore.getState().project?.media[source.media];
    if (existing) return existing;
  }
  const reply = await transport.send(cmd("Media", { type: "Import", id: newId(), source }), opts);
  if (reply.type !== "Media") throw new Error(`unexpected reply to Media::Import: ${reply.type}`);
  return reply.media;
}

/** Default `waitForMediaLength` timeout. */
export const MEDIA_LENGTH_TIMEOUT_MS = 60_000;

/**
 * Resolves with the project's `MediaRef` once its length is known (`frames > 0`): at once
 * for most files; for media imported with an unknown length, when the engine's background
 * decode patches it in (a `Media` entity upsert in the project mirror). Event-driven: it
 * watches the project store and the transport's `Media` events, no polling.
 *
 * Rejects if the media is removed or reported `Missing`, or after `timeoutMs`.
 * `onProgress` gets the engine's `ImportProgress` (0..1) for this media.
 */
export function waitForMediaLength(
  transport: EngineTransport,
  media: MediaId,
  opts: { timeoutMs?: number; onProgress?: (progress: number) => void } = {},
): Promise<MediaRef> {
  const current = () => useProjectStore.getState().project?.media[media];
  const now = current();
  if (now && now.frames > 0) return Promise.resolve(now);
  return new Promise<MediaRef>((resolve, reject) => {
    let seen = now !== undefined;
    const fail = (message: string) => {
      cleanup();
      reject(new Error(message));
    };
    const check = () => {
      const m = current();
      if (m && m.frames > 0) {
        cleanup();
        resolve(m);
      } else if (m) {
        seen = true;
      } else if (seen) {
        fail("the media was removed while loading");
      }
    };
    const timer = setTimeout(() => fail("timed out waiting for the media to load"), opts.timeoutMs ?? MEDIA_LENGTH_TIMEOUT_MS);
    const offStore = useProjectStore.subscribe(check);
    const offEvents = transport.onEvent((e) => {
      if (e.type !== "Media" || !("media" in e.event) || e.event.media !== media) return;
      if (e.event.type === "ImportProgress") opts.onProgress?.(e.event.progress);
      else if (e.event.type === "Missing") fail("the media file is missing or could not be decoded");
    });
    function cleanup() {
      clearTimeout(timer);
      offStore();
      offEvents();
    }
  });
}
