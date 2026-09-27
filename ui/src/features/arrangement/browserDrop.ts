/**
 * Accepting sample-browser drops. The payload format (MIME type, JSON shape, import helper)
 * is owned by the browser feature: see `ui/src/features/browser/dragPayload.ts`.
 */
import type { Beats, ClipId, TrackId } from "@/generated";
import {
  resolveDroppedMedia,
  waitForMediaLength,
  type BrowserDragPayload,
} from "@/features/browser/dragPayload";
import { cmd, newId, nextGestureId, type EngineTransport } from "@/transport";
import { asOneStep } from "./editMath";
import { useArrangementUi, type PendingImport } from "./uiStore";

export {
  BROWSER_DRAG_MIME,
  hasBrowserDrag,
  readBrowserDrag,
  resolveDroppedMedia,
  type BrowserDragPayload,
} from "@/features/browser/dragPayload";

/** How long a failed import's placeholder stays up. */
export const IMPORT_ERROR_MS = 4000;

export interface DropMediaOptions {
  /** `waitForMediaLength` timeout (default: its own). */
  timeoutMs?: number;
}

/**
 * Create an audio clip from a browser drop at `at` on `track` (`null`: on a new audio
 * track). Import (if needed), track creation and clip are ONE undo step: all commands share
 * one gesture id, closed with `Edit::EndGesture`. Media whose length is unknown at import
 * (VBR) is waited for (event-driven, with a timeout) before the clip is created. While it
 * runs, a pending import is shown in the lane (`useArrangementUi().imports`); on failure
 * it shows the error for `IMPORT_ERROR_MS`, and the promise rejects.
 */
export async function dropBrowserMedia(
  transport: EngineTransport,
  payload: BrowserDragPayload,
  target: { track: TrackId | null; at: Beats },
  opts: DropMediaOptions = {},
): Promise<ClipId> {
  const ui = useArrangementUi.getState;
  const pending: PendingImport = { id: newId(), track: target.track, at: target.at, name: payload.name, progress: null, error: null };
  const gesture = nextGestureId();
  ui().putImport(pending);
  try {
    const media = await resolveDroppedMedia(transport, payload, { gesture });
    if (media.frames === 0) {
      await waitForMediaLength(transport, media.id, {
        timeoutMs: opts.timeoutMs,
        onProgress: (progress) => ui().putImport({ ...pending, progress }),
      });
    }
    const clip = { id: newId(), start: target.at, media: media.id };
    if (target.track) {
      await transport.send(cmd("Clip", { type: "CreateAudio", track: target.track, ...clip }), { gesture });
    } else {
      const track = newId();
      const command = asOneStep("Add Audio Clip", [
        cmd("Track", { type: "Create", id: track, kind: "Audio", name: null, color: null, parent: null, before: null }),
        cmd("Clip", { type: "CreateAudio", track, ...clip }),
      ])!;
      await transport.send(command, { gesture });
    }
    ui().removeImport(pending.id);
    return clip.id;
  } catch (err) {
    ui().putImport({ ...pending, error: err instanceof Error ? err.message : String(err) });
    setTimeout(() => ui().removeImport(pending.id), IMPORT_ERROR_MS);
    throw err;
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
}
