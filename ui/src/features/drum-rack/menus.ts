/**
 * "slice to drum rack" in the context menus: an audio clip's (its sample, on a new MIDI
 * track right below) and a browser sample's (a new track, or the selected track's drum rack).
 * Each is one undo step.
 */

import type { Clip, LibraryItem, Project } from "@/generated";
import { libraryItemPayload } from "@/features/browser/dragPayload";
import { notify } from "@/features/notifications";
import { t } from "@/i18n";
import { errorMessage } from "@/features/transport-bar/engine";
import type { ContextMenuEntry } from "@/kit";
import { devicesOfTrack, useProjectStore, useSelectionStore } from "@/state";
import type { EngineTransport } from "@/transport";
import { isDrumRack } from "./padUtils";
import { firstFreeNote, sliceMediaToNewTrack, slicePayloadToNewTrack, slicePayloadToPads } from "./sliceActions";

const failed = (name: string) => (e: unknown) => notify("Error", t("drum_rack.slice.failed", { name, error: errorMessage(e) }));

/** An audio clip's entry; `[]` for other clips (MIDI, take lanes, missing media). */
export function sliceClipEntries(transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  if (clip.content.type !== "Audio" || clip.lane) return [];
  const project = useProjectStore.getState().project;
  const media = clip.content.media;
  if (!project?.media[media]) return [];
  return [
    {
      label: t("drum_rack.slice.to_drum_rack"),
      onSelect: () => void sliceMediaToNewTrack(transport, project, media, { name: clip.name || null, after: clip.track }).catch(failed(clip.name)),
    },
  ];
}

/** `entries` with the clip's slice entry in the group before the last one (the deletes). */
export function withSliceClipEntries(entries: ReadonlyArray<ContextMenuEntry>, transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  const extra = sliceClipEntries(transport, clip);
  if (extra.length === 0) return [...entries];
  const lastSep = entries.lastIndexOf("separator");
  if (lastSep < 0) return entries.length ? [...entries, "separator", ...extra] : extra;
  return [...entries.slice(0, lastSep), ...extra, ...entries.slice(lastSep)];
}

/** The selected track's first drum rack (on its own chain). */
function selectedRack(project: Project) {
  const track = useSelectionStore.getState().selectedTrack;
  return track ? devicesOfTrack(project, track).find(isDrumRack) : undefined;
}

/** A browser audio sample's entries: to a new drum rack track, or onto the selected track's drum rack. */
export function sliceSampleEntries(transport: EngineTransport | null, item: LibraryItem): ContextMenuEntry[] {
  const payload = item.kind === "Audio" ? libraryItemPayload(item) : null;
  const project = useProjectStore.getState().project;
  if (!payload || !transport || !project) return [];
  const rack = selectedRack(project);
  const entries: ContextMenuEntry[] = [
    {
      label: t("drum_rack.slice.to_new_rack"),
      onSelect: () => void slicePayloadToNewTrack(transport, project, payload).catch(failed(item.name)),
    },
  ];
  if (rack) {
    entries.push({
      label: t("drum_rack.slice.to_selected_rack"),
      disabled: firstFreeNote(project, rack.id) === null,
      onSelect: () => void slicePayloadToPads(transport, project, rack.id, payload).catch(failed(item.name)),
    });
  }
  return entries;
}
