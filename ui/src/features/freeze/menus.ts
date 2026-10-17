/**
 * Freeze entries of the arrangement's context menus, inserted before the menu's last group
 * (the destructive "Delete" entries), like the clip-editing entries.
 * - Track header: Freeze / Unfreeze / Flatten (the selected tracks; Flatten asks first, #717),
 *   Cancel while rendering.
 * - Clip: Bounce to New Track, Bounce in Place (audio), Consolidate (the selected clips).
 * - Device: freeze up to here (#25: partial chain freeze), unfreeze on a frozen device.
 * - Time selection: Bounce to New Track (the selected range of each selected track).
 */

import type { Beats, Clip, Device, DeviceCategory, Track, TrackId } from "@/generated";
import { useConfirm, type ContextMenuEntry } from "@/kit";
import { devicesOfTrack, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { frozenAncestor, isDeviceFrozen, isFrozenTrack } from "./inert";
import type { EngineTransport } from "@/transport";
import { t } from "@/i18n";
import {
  bounceClips,
  bounceRange,
  canFreeze,
  cancelRenderJob,
  consolidateClips,
  flattenTracks,
  freezeTracks,
  freezeUpTo,
  jobOfTrack,
  tracksWithClipsIn,
  unfreezeTracks,
  useFreezeUi,
} from "./store";

function insertBeforeLastGroup(entries: ReadonlyArray<ContextMenuEntry>, extra: ContextMenuEntry[]): ContextMenuEntry[] {
  if (extra.length === 0) return [...entries];
  const lastSep = entries.lastIndexOf("separator");
  if (lastSep < 0) return entries.length ? [...entries, "separator", ...extra] : extra;
  return [...entries.slice(0, lastSep), "separator", ...extra, ...entries.slice(lastSep)];
}

const freezable = (t: Track) => t.kind === "Audio" || t.kind === "Midi" || t.kind === "Group";

/** "track" or "group" in the labels of `tracks` (all groups: "group"). */
const noun = (tracks: ReadonlyArray<Track>) => (tracks.length > 0 && tracks.every((t) => t.kind === "Group") ? "group" : "track");

/**
 * #717: flatten replaces the tracks' clips and devices with their render, for good once the
 * undo history no longer reaches it: ask first.
 */
export function confirmFlatten(transport: EngineTransport, tracks: ReadonlyArray<Track>): void {
  const groups = tracks.some((t) => t.kind === "Group");
  useConfirm.getState().ask({
    title:
      tracks.length === 1
        ? t("freeze.flatten_confirm.title_one", { name: tracks[0]!.name })
        : t("freeze.flatten_confirm.title_many", { kind: noun(tracks), count: tracks.length }),
    body: t("freeze.flatten_confirm.body", { count: tracks.length, groups: groups ? "yes" : "no" }),
    action: t("freeze.flatten_confirm.action"),
    danger: true,
    run: () =>
      void flattenTracks(
        transport,
        tracks.map((t) => t.id),
      ),
  });
}

/** Track-menu entries for `track` (acting on `selected` when it contains `track`). */
export function freezeTrackEntries(transport: EngineTransport, track: Track, selected: ReadonlyArray<TrackId>): ContextMenuEntry[] {
  if (!freezable(track)) return [];
  const p = useProjectStore.getState().project;
  // Inside a frozen group: locked until the group is unfrozen.
  const owner = p && frozenAncestor(p.tracks, track.id);
  if (owner) return [{ label: t("freeze.menu.frozen_with_group", { name: owner.name }), disabled: true, onSelect: () => {} }];
  const ids = selected.includes(track.id) ? selected : [track.id];
  const tracks = ids.map((id) => p?.tracks[id]).filter((t): t is Track => !!t && freezable(t));
  const job = jobOfTrack(useFreezeUi.getState(), track.id);
  if (job) {
    return [
      {
        label: t(job.progress === null ? "freeze.menu.cancel_queued" : "freeze.menu.cancel", { kind: job.kind.toLowerCase() }),
        onSelect: () => void cancelRenderJob(transport, job),
      },
    ];
  }
  const busy = (id: TrackId) => !!jobOfTrack(useFreezeUi.getState(), id);
  if (track.freeze) {
    const frozenTracks = tracks.filter((t) => t.freeze);
    const frozen = frozenTracks.map((t) => t.id);
    const n = noun(frozenTracks);
    return [
      {
        label: t("freeze.menu.unfreeze", { kind: n, count: Math.max(1, frozen.length) }),
        onSelect: () => void unfreezeTracks(transport, frozen),
      },
      {
        label: t("freeze.menu.flatten", { kind: n, count: Math.max(1, frozen.length) }),
        onSelect: () => confirmFlatten(transport, frozenTracks),
      },
    ];
  }
  const ready = tracks.filter((t) => canFreeze(t.id) && !busy(t.id));
  const n = noun(tracks);
  return [
    {
      label: t("freeze.menu.freeze", { kind: n, count: Math.max(1, ready.length) }),
      disabled: !ready.some((t) => t.id === track.id),
      onSelect: () =>
        void freezeTracks(
          transport,
          ready.map((t) => t.id),
        ),
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

/**
 * Device-menu entries (#25): "freeze up to here" renders the track through its chain up to
 * and including `device`, the devices after it staying live. Offered on the track-chain
 * devices of audio, MIDI and group tracks, from the instrument on: never on a MIDI effect
 * (nor on anything before a MIDI track's instrument), nor inside racks or drum pads. On a
 * frozen device: "unfreeze track".
 */
export function freezeDeviceEntries(transport: EngineTransport, device: Device, categoryOf: (d: Device) => DeviceCategory): ContextMenuEntry[] {
  const p = useProjectStore.getState().project;
  const track = p?.tracks[device.track];
  if (!p || !track || !freezable(track) || device.pad || device.chain) return [];
  if (categoryOf(device) === "NoteEffect") return [];
  const chain = devicesOfTrack(p, track.id);
  const at = chain.findIndex((d) => d.id === device.id);
  if (at < 0) return [];
  const instrument = track.kind === "Midi" ? chain.findIndex((d) => categoryOf(d) === "Instrument") : -1;
  if (at < instrument) return [];
  // Inside a frozen group, or rendering: the track menu says why / cancels.
  if (frozenAncestor(p.tracks, track.id) || jobOfTrack(useFreezeUi.getState(), track.id)) return [];
  if (track.freeze) {
    if (!isDeviceFrozen(p, device)) return [];
    return [{ label: t("freeze.menu.unfreeze", { kind: noun([track]), count: 1 }), onSelect: () => void unfreezeTracks(transport, [track.id]) }];
  }
  return [
    {
      label: t("freeze.menu.freeze_up_to"),
      disabled: !canFreeze(track.id),
      onSelect: () => void freezeUpTo(transport, track.id, device.id),
    },
  ];
}

/** The selected arrangement clips (main lanes), `clip` included. */
function selectedClips(clip: Clip): Clip[] {
  const p = useProjectStore.getState().project;
  if (!p) return [clip];
  const out = [...itemSelection.getState().selected.clip].map((id) => p.clips[id]).filter((c): c is Clip => !!c && !c.lane);
  return out.some((c) => c.id === clip.id) ? out : [clip];
}

/** Clip-menu entries for `clip` (acting on the selected clips). */
export function freezeClipEntries(transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  const p = useProjectStore.getState().project;
  const clips = selectedClips(clip);
  const tracks = clips.map((c) => p?.tracks[c.track]);
  if (tracks.some((t) => !t || (t.kind !== "Audio" && t.kind !== "Midi"))) return [];
  const frozen = tracks.some((t) => t && p && isFrozenTrack(p, t));
  const busy = clips.some((c) => !!jobOfTrack(useFreezeUi.getState(), c.track));
  const allAudio = tracks.every((t) => t?.kind === "Audio");
  return [
    {
      label: t("freeze.menu.bounce_new"),
      disabled: busy || (!!p && tracks.some((t) => t && frozenAncestor(p.tracks, t.id))),
      onSelect: () => void bounceClips(transport, clips, "NewTrack"),
    },
    ...(allAudio
      ? [
          {
            label: t("freeze.menu.bounce_in_place"),
            disabled: busy || frozen,
            onSelect: () => void bounceClips(transport, clips, "InPlace"),
          },
        ]
      : []),
    {
      label: t("freeze.menu.consolidate"),
      disabled: busy || frozen,
      onSelect: () => void consolidateClips(transport, clips),
    },
  ];
}

export function withFreezeClipEntries(entries: ReadonlyArray<ContextMenuEntry>, transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  return insertBeforeLastGroup(entries, freezeClipEntries(transport, clip));
}

/**
 * Time-selection menu entries: "Bounce to New Track" renders `[start, end)` of each selected
 * audio/MIDI track (`tracks`: groups already expanded to their children) to a new track.
 */
export function freezeRangeEntries(transport: EngineTransport, start: Beats, end: Beats, tracks: ReadonlyArray<TrackId>): ContextMenuEntry[] {
  const p = useProjectStore.getState().project;
  const content = tracks.filter((id) => {
    const k = p?.tracks[id]?.kind;
    return k === "Audio" || k === "Midi";
  });
  if (!content.length) return [];
  const withClips = tracksWithClipsIn(content, start, end);
  const busy = withClips.some((id) => !!jobOfTrack(useFreezeUi.getState(), id));
  const inert = !!p && withClips.some((id) => frozenAncestor(p.tracks, id));
  return [
    {
      label: t("freeze.menu.bounce_new"),
      disabled: !withClips.length || busy || inert,
      onSelect: () => void bounceRange(transport, start, end, withClips),
    },
  ];
}
