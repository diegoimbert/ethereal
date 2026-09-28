/**
 * Importing audio from the user's computer (CONTRACTS.md §12.13), for every entry point:
 * the "Import audio…" command (⌘I), OS drops on the arrangement lanes and on the browser.
 *
 * - A `File` (web, remote UI, desktop fallback) is uploaded in chunks (`BeginUpload` →
 *   `UploadChunk`s → `Import { source: Upload }`); a desktop path is imported by the
 *   engine (`Import { source: Path }`). Media whose length is unknown at import (VBR) is
 *   waited for before its clip is created.
 * - Several files at once: imported one after the other, each with its own status row
 *   (`useImportStore`: progress, cancel, error) and, when placed in the arrangement, a lane
 *   placeholder. A file that fails doesn't stop the others.
 * - Placement (`target`): on a track, the clips go one after the other from `at`; with
 *   `track: null` (below the tracks), each file gets a new audio track, all at `at`.
 *   No target: the media is only added to the project (the browser's project media).
 * - One undo step for the whole gesture: every command shares one gesture id, closed with
 *   `Edit::EndGesture`.
 * - In a collab session the engine pushes the imported media's bytes to every peer.
 */
import type { Beats, ClipId, GestureId, MediaRef, TrackId } from "@/generated";
import { asOneStep } from "@/features/arrangement/editMath";
import { useArrangementUi } from "@/features/arrangement/uiStore";
import { waitForMediaLength } from "@/features/browser/dragPayload";
import { uploadFile } from "@/features/remote/upload";
import { useProjectStore } from "@/state";
import { cmd, isCommandFailed, newId, nextGestureId, type EngineTransport } from "@/transport";
import { useImportStore } from "./importStore";
import { rejectReason, sourceName, type ImportSource } from "./sources";

export interface ImportTarget {
  /** Audio track to place the clips on; `null` = a new audio track per file. */
  track: TrackId | null;
  at: Beats;
}

export interface ImportOutcome {
  source: ImportSource;
  media: MediaRef | null;
  clip: ClipId | null;
  /** `null` on success; `"cancelled"` when the user cancelled it. */
  error: string | null;
}

/** How long a failed import's lane placeholder stays up. */
export const PLACEHOLDER_ERROR_MS = 4000;

export interface ImportOptions {
  /** `waitForMediaLength` timeout (default: its own). */
  timeoutMs?: number;
}

function message(e: unknown): string {
  if (isCommandFailed(e)) return e.code === "Decode" ? "not a readable audio file" : e.error.message;
  return e instanceof Error ? e.message : String(e);
}

/** Import one source into the project (no clip). */
async function importOne(
  transport: EngineTransport,
  source: ImportSource,
  gesture: GestureId,
  signal: AbortSignal,
  onUpload: (fraction: number) => void,
): Promise<MediaRef> {
  if (source.kind === "file") {
    const size = source.file.size;
    return uploadFile(transport, source.file, (sent) => onUpload(size > 0 ? sent / size : 1), { gesture, signal });
  }
  const reply = await transport.send(cmd("Media", { type: "Import", id: newId(), source: { type: "Path", path: source.path } }), { gesture });
  if (reply.type !== "Media") throw new Error(`unexpected reply ${reply.type}`);
  return reply.media;
}

/** Import `sources` (and place them at `target`); see the module docs. Never rejects. */
export async function importAudio(
  transport: EngineTransport,
  sources: readonly ImportSource[],
  target: ImportTarget | null = null,
  opts: ImportOptions = {},
): Promise<ImportOutcome[]> {
  const store = useImportStore.getState;
  const lanes = useArrangementUi.getState;
  const jobs = sources.map((source) => ({ source, id: newId(), abort: new AbortController() }));
  for (const j of jobs) {
    const reason = rejectReason(j.source);
    store().put({
      id: j.id,
      name: sourceName(j.source),
      phase: "queued",
      progress: null,
      error: reason ? `${sourceName(j.source)}: ${reason}` : null,
      cancel: reason ? null : () => j.abort.abort(),
    });
  }
  const gesture = nextGestureId();
  const out: ImportOutcome[] = [];
  let at = target?.at ?? 0;
  try {
    for (const { source, id, abort } of jobs) {
      const name = sourceName(source);
      const reason = rejectReason(source);
      if (reason) {
        out.push({ source, media: null, clip: null, error: reason });
        continue;
      }
      if (abort.signal.aborted) {
        store().remove(id);
        out.push({ source, media: null, clip: null, error: "cancelled" });
        continue;
      }
      const lane = target ? { id, track: target.track, at, name, progress: null as number | null, error: null as string | null } : null;
      if (lane) lanes().putImport(lane);
      const progress = (phase: "uploading" | "decoding" | "placing", p: number | null) => {
        store().update(id, { phase, progress: p });
        if (lane) lanes().putImport({ ...lane, progress: p });
      };
      try {
        progress(source.kind === "file" ? "uploading" : "decoding", source.kind === "file" ? 0 : null);
        let media = await importOne(transport, source, gesture, abort.signal, (f) => progress("uploading", f));
        // Past this point the media is in the project: the import can't be cancelled.
        store().update(id, { cancel: null });
        if (media.frames === 0) {
          progress("decoding", null);
          media = await waitForMediaLength(transport, media.id, {
            timeoutMs: opts.timeoutMs,
            onProgress: (p) => progress("decoding", p),
          });
        }
        let clip: ClipId | null = null;
        if (target) {
          progress("placing", null);
          clip = newId();
          const c = { id: clip, start: at, media: media.id };
          if (target.track) {
            await transport.send(cmd("Clip", { type: "CreateAudio", track: target.track, ...c }), { gesture });
            const placed = useProjectStore.getState().project?.clips[clip];
            if (placed) at = placed.start + placed.length;
          } else {
            const track = newId();
            const command = asOneStep("Import audio", [
              cmd("Track", { type: "Create", id: track, kind: "Audio", name: null, color: null, parent: null, before: null }),
              cmd("Clip", { type: "CreateAudio", track, ...c }),
            ])!;
            await transport.send(command, { gesture });
          }
        }
        store().remove(id);
        if (lane) lanes().removeImport(id);
        out.push({ source, media, clip, error: null });
      } catch (e) {
        if (abort.signal.aborted) {
          store().remove(id);
          if (lane) lanes().removeImport(id);
          out.push({ source, media: null, clip: null, error: "cancelled" });
          continue;
        }
        const why = message(e);
        store().update(id, { error: `${name}: ${why}`, cancel: null });
        if (lane) {
          lanes().putImport({ ...lane, error: why });
          setTimeout(() => lanes().removeImport(id), PLACEHOLDER_ERROR_MS);
        }
        out.push({ source, media: null, clip: null, error: why });
      }
    }
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
  return out;
}

/**
 * Where "Import audio…" places files: on the selected audio track, else on new tracks, at
 * the playhead.
 */
export function commandTarget(selectedTrack: TrackId | null, playhead: Beats): ImportTarget {
  const t = selectedTrack ? useProjectStore.getState().project?.tracks[selectedTrack] : undefined;
  return { track: t?.kind === "Audio" ? t.id : null, at: Math.max(0, playhead) };
}
