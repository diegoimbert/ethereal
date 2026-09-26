/**
 * Accepting sample-browser drops.
 *
 * The payload format is defined by the browser feature (ui-shell,
 * `ui/src/features/browser/dragPayload.ts`): MIME `application/x-ethereal-media+json`, JSON
 * `{ version: 1, kind: "media", source: MediaSource, name, file_kind }`. It is not on `main`
 * yet, so this file reads the same documented format; once it lands, these helpers become
 * re-exports of `@/features/browser/dragPayload`.
 */

import type { MediaRef, MediaSource } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";

export const BROWSER_DRAG_MIME = "application/x-ethereal-media+json";

export interface BrowserDragPayload {
  version: 1;
  kind: "media";
  source: MediaSource;
  name: string;
  file_kind: string;
}

type DragData = Pick<DataTransfer, "types" | "getData">;

/** Whether a drag carries a browser payload (usable during `dragover`). */
export function hasBrowserDrag(dt: Pick<DataTransfer, "types"> | null | undefined): boolean {
  return !!dt && Array.from(dt.types).includes(BROWSER_DRAG_MIME);
}

/** The payload of a drop, or `null` if absent/malformed. */
export function readBrowserDrag(dt: DragData | null | undefined): BrowserDragPayload | null {
  if (!hasBrowserDrag(dt)) return null;
  try {
    const v = JSON.parse(dt!.getData(BROWSER_DRAG_MIME)) as Partial<BrowserDragPayload> | null;
    if (!v || v.version !== 1 || v.kind !== "media" || typeof v.name !== "string") return null;
    const s = v.source;
    const ok =
      !!s &&
      ((s.type === "Location" && typeof s.path === "string" && typeof s.location === "object" && s.location !== null) ||
        (s.type === "Project" && typeof s.media === "string"));
    return ok ? (v as BrowserDragPayload) : null;
  } catch {
    return null;
  }
}

/** The project media for a payload, importing it first unless it is already in the project. */
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
