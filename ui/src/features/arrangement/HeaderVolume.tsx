import { useRef } from "react";
import type { Track } from "@/generated";
import { useGestureSender } from "@/features/devices/gesture";
import { paramToPlain } from "@/domain/paramScale";
import { useLiveAutomation } from "@/features/automation/live";
import { VOLUME_INFO } from "@/features/automation/params";
import { midiTarget } from "@/features/midi-learn/targets";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useT } from "@/i18n";
import { groupVolumes, type VolumeSnapshot } from "./groupVolume";
import { useArrangementUi } from "./uiStore";
import { VolumeFader } from "./VolumeFader";

/**
 * Compact horizontal volume fader in a track header (the mixer view is gone). Drag left /
 * right (Shift: fine) as one undo step; arrows step it; double-click resets to 0 dB. The
 * dB value shows while hovering or dragging. When this track is part of a multi-track
 * selection, dragging or stepping moves every selected track by the same dB amount (their
 * balance is kept, see `groupVolumes`).
 */
export function HeaderVolume({ track }: { track: Track }) {
  const t = useT();
  const sender = useGestureSender();
  const base = track.mixer.volume;
  // Volume automation: the fader follows what the engine plays.
  const live = useLiveAutomation({ type: "TrackVolume", track: track.id });
  const db = live === null ? base : paramToPlain(VOLUME_INFO, live);
  /** The selection's volumes at the drag's start (a multi-track drag keeps their balance). */
  const group = useRef<VolumeSnapshot | null>(null);

  /** Volumes of the selected tracks when this track is part of a multi-track selection. */
  const selectionSnapshot = (): VolumeSnapshot | null => {
    const selected = useArrangementUi.getState().selectedTracks;
    const tracks = useProjectStore.getState().project?.tracks;
    if (!tracks || selected.size < 2 || !selected.has(track.id)) return null;
    return new Map([...selected].flatMap((id) => (tracks[id] ? [[id, tracks[id].mixer.volume] as const] : [])));
  };

  const set = (v: number, snapshot: VolumeSnapshot | null) => {
    if (!snapshot) {
      if (v !== base) void sender.send(cmd("Mixer", { type: "SetVolume", track: track.id, volume: v }));
      return;
    }
    const tracks = useProjectStore.getState().project?.tracks;
    for (const [id, volume] of groupVolumes(snapshot, track.id, v)) {
      if (tracks?.[id] && tracks[id].mixer.volume !== volume) void sender.send(cmd("Mixer", { type: "SetVolume", track: id, volume }));
    }
  };

  return (
    <VolumeFader
      db={db}
      label={t("arrangement.header.volume", { name: track.name })}
      attrs={midiTarget({ type: "Param", target: { type: "TrackVolume", track: track.id } })}
      onBegin={() => {
        group.current = selectionSnapshot();
        sender.begin();
      }}
      onEnd={() => {
        group.current = null;
        sender.end();
      }}
      // A drag moves the selection captured at its start, a key the current one; a reset
      // (double-click) only this track.
      onChange={(v, from) => set(v, from === "drag" ? group.current : from === "key" ? selectionSnapshot() : null)}
    />
  );
}
