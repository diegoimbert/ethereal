/**
 * Freeze entries of the arrangement's context menus, inserted before the menu's last group
 * (the destructive "Delete" entries), like the clip-editing entries.
 * - Track header: Freeze / Unfreeze / Flatten (the selected tracks), Cancel while rendering.
 * - Clip: Bounce to New Track, Bounce in Place (audio), Consolidate (the selected clips).
 */

import type { Clip, Track, TrackId } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import type { EngineTransport } from "@/transport";
import {
  bounceClips,
  canFreeze,
  cancelRenderJob,
  consolidateClips,
  flattenTracks,
  freezeTracks,
  jobOfTrack,
  unfreezeTracks,
  useFreezeUi,
} from "./store";

function insertBeforeLastGroup(entries: ReadonlyArray<ContextMenuEntry>, extra: ContextMenuEntry[]): ContextMenuEntry[] {
  if (extra.length === 0) return [...entries];
  const lastSep = entries.lastIndexOf("separator");
  if (lastSep < 0) return entries.length ? [...entries, "separator", ...extra] : extra;
  return [...entries.slice(0, lastSep), "separator", ...extra, ...entries.slice(lastSep)];
}

const plural = (n: number, one: string, many: string) => (n > 1 ? `${many.replace("#", String(n))}` : one);

/** Track-menu entries for `track` (acting on `selected` when it contains `track`). */
export function freezeTrackEntries(transport: EngineTransport, track: Track, selected: ReadonlyArray<TrackId>): ContextMenuEntry[] {
  if (track.kind !== "Audio" && track.kind !== "Midi") return [];
  const p = useProjectStore.getState().project;
  const ids = selected.includes(track.id) ? selected : [track.id];
  const tracks = ids.map((id) => p?.tracks[id]).filter((t): t is Track => !!t && (t.kind === "Audio" || t.kind === "Midi"));
  const job = jobOfTrack(useFreezeUi.getState(), track.id);
  if (job) {
    return [
      {
        label: job.progress === null ? `Cancel Queued ${job.kind}` : `Cancel ${job.kind}`,
        onSelect: () => void cancelRenderJob(transport, job),
      },
    ];
  }
  const busy = (id: TrackId) => !!jobOfTrack(useFreezeUi.getState(), id);
  if (track.freeze) {
    const frozen = tracks.filter((t) => t.freeze).map((t) => t.id);
    return [
      {
        label: plural(frozen.length, "Unfreeze Track", "Unfreeze # Tracks"),
        onSelect: () => void unfreezeTracks(transport, frozen),
      },
      {
        label: plural(frozen.length, "Flatten Track", "Flatten # Tracks"),
        onSelect: () => void flattenTracks(transport, frozen),
      },
    ];
  }
  const freezable = tracks.filter((t) => canFreeze(t.id) && !busy(t.id)).map((t) => t.id);
  return [
    {
      label: plural(freezable.length, "Freeze Track", "Freeze # Tracks"),
      disabled: !freezable.includes(track.id),
      onSelect: () => void freezeTracks(transport, freezable),
    },
  ];
}

export function withFreezeTrackEntries(
  entries: ReadonlyArray<ContextMenuEntry>,
  transport: EngineTransport,
  track: Track,
  selected: ReadonlyArray<TrackId>,
): ContextMenuEntry[] {
  return insertBeforeLastGroup(entries, freezeTrackEntries(transport, track, selected));
}

/** The selected arrangement clips (main lanes), `clip` included. */
function selectedClips(clip: Clip): Clip[] {
  const p = useProjectStore.getState().project;
  if (!p) return [clip];
  const out = [...itemSelection.getState().selected.clip]
    .map((id) => p.clips[id])
    .filter((c): c is Clip => !!c && !c.lane);
  return out.some((c) => c.id === clip.id) ? out : [clip];
}

/** Clip-menu entries for `clip` (acting on the selected clips). */
export function freezeClipEntries(transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  const p = useProjectStore.getState().project;
  const clips = selectedClips(clip);
  const tracks = clips.map((c) => p?.tracks[c.track]);
  if (tracks.some((t) => !t || (t.kind !== "Audio" && t.kind !== "Midi"))) return [];
  const frozen = tracks.some((t) => t?.freeze);
  const busy = clips.some((c) => !!jobOfTrack(useFreezeUi.getState(), c.track));
  const allAudio = tracks.every((t) => t?.kind === "Audio");
  return [
    {
      label: "Bounce to New Track",
      disabled: busy,
      onSelect: () => void bounceClips(transport, clips, "NewTrack"),
    },
    ...(allAudio
      ? [
          {
            label: "Bounce in Place",
            disabled: busy || frozen,
            onSelect: () => void bounceClips(transport, clips, "InPlace"),
          },
        ]
      : []),
    {
      label: "Consolidate",
      disabled: busy || frozen,
      onSelect: () => void consolidateClips(transport, clips),
    },
  ];
}

export function withFreezeClipEntries(
  entries: ReadonlyArray<ContextMenuEntry>,
  transport: EngineTransport,
  clip: Clip,
): ContextMenuEntry[] {
  return insertBeforeLastGroup(entries, freezeClipEntries(transport, clip));
}
