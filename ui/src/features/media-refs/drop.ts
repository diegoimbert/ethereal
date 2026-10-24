/**
 * Dropping a sample onto a missing one (an audio clip whose sample is missing, a missing
 * file's row in the relink / resolve dialogs) resolves it to the dropped file:
 *
 * - a sample from the app's browser (its `MediaSource`; media already in the project
 *   relinks to that media's file);
 * - an OS file: on the desktop (macOS) the shell forwards its path (`noteHover`, the engine
 *   reads it); elsewhere the page gets a `File`, uploaded to the engine's staging.
 *
 * The drop target then relinks (`MediaRef::Relink`) or picks it like "choose file…".
 */
import { useState, type DragEvent } from "react";
import type { MediaId, MediaSource } from "@/generated";
import { hasBrowserDrag, hasBrowserMidiDrag, readBrowserDrag, type BrowserDragPayload, type DragData } from "@/features/browser/dragPayload";
import { droppedFiles, hasOsFiles, noteHover, type FileDragData } from "@/features/import/osDrop";
import { rejectReason, sourceName, type ImportSource } from "@/features/import/sources";
import { useProjectStore } from "@/state";
import { notify } from "@/features/notifications";
import { cmd, type EngineTransport } from "@/transport";
import { t } from "@/i18n";
import { sourceLabel, stageUpload } from "./sources";
import { errorText } from "./store";

/** What was dropped: a browser sample, or OS files (the first audio one is used). */
export type SampleDrop = { kind: "browser"; payload: BrowserDragPayload } | { kind: "files"; sources: ImportSource[] };

type Drag = DragData & FileDragData;

/** Can this drag resolve a missing sample? (During `dragover` only the types are readable.) */
export function isSampleDrag(dt: Pick<Drag, "types"> | null | undefined): boolean {
  return hasOsFiles(dt) || (hasBrowserDrag(dt) && !hasBrowserMidiDrag(dt));
}

/** The sample of a drop, or `null` (nothing usable). */
export function readSampleDrop(dt: Drag | null | undefined): SampleDrop | null {
  if (hasOsFiles(dt)) {
    const sources = droppedFiles(dt);
    return sources.length > 0 ? { kind: "files", sources } : null;
  }
  const payload = readBrowserDrag(dt);
  return payload && payload.file_kind === "Audio" ? { kind: "browser", payload } : null;
}

/** A source `Relink` reads: project media are relinked to their file, not to the media. */
export function relinkableSource(source: MediaSource): MediaSource {
  if (source.type !== "Project") return source;
  const m = useProjectStore.getState().project?.media[source.media];
  if (!m) throw new Error(t("media_refs.drop.not_audio"));
  if (m.location.type === "External") return { type: "Path", path: m.location.path };
  return { type: "Location", location: { type: "ProjectMedia" }, path: m.file.replace(/^media\//, "") };
}

/**
 * The engine source of a drop (and its display text): a browser sample's source, an OS
 * path as is, a `File` uploaded first. Rejects with a readable error for a non-audio file.
 */
export async function sampleDropSource(transport: EngineTransport, drop: SampleDrop): Promise<{ source: MediaSource; display: string }> {
  if (drop.kind === "browser") {
    const source = relinkableSource(drop.payload.source);
    return { source, display: sourceLabel(source) };
  }
  const s = drop.sources.find((x) => rejectReason(x) === null);
  if (!s) throw new Error(rejectReason(drop.sources[0]!) ?? t("media_refs.drop.not_audio"));
  if (s.kind === "path") return { source: { type: "Path", path: s.path }, display: s.path };
  return { source: await stageUpload(transport, s.file), display: sourceName(s) };
}

/** Relink `media` to a dropped sample (one undo step); a failure is shown as a notice. */
export async function relinkDrop(transport: EngineTransport, media: MediaId, drop: SampleDrop): Promise<boolean> {
  try {
    const { source } = await sampleDropSource(transport, drop);
    await transport.send(cmd("MediaRef", { type: "Relink", media, source }));
    return true;
  } catch (e) {
    notify("Error", errorText(e));
    return false;
  }
}

/** Drop handlers of a missing sample's row, and whether a sample hovers it (the highlight). */
export function useSampleDrop(onDrop: (drop: SampleDrop) => void): {
  over: boolean;
  props: {
    onDragOver: (e: DragEvent<HTMLElement>) => void;
    onDragLeave: (e: DragEvent<HTMLElement>) => void;
    onDrop: (e: DragEvent<HTMLElement>) => void;
  };
} {
  const [over, setOver] = useState(false);
  return {
    over,
    props: {
      onDragOver: (e) => {
        if (!isSampleDrag(e.dataTransfer)) return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = "copy";
        setOver(true);
        // Desktop: the shell delivers the paths, not the drop.
        if (hasOsFiles(e.dataTransfer))
          noteHover(e, (sources) => {
            setOver(false);
            onDrop({ kind: "files", sources });
          });
      },
      onDragLeave: (e) => {
        if (e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget)) return;
        setOver(false);
      },
      onDrop: (e) => {
        setOver(false);
        const drop = readSampleDrop(e.dataTransfer);
        if (!drop) return;
        e.preventDefault();
        e.stopPropagation();
        onDrop(drop);
      },
    },
  };
}
