import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import type { Track } from "@/generated";
import { useGestureSender } from "@/features/devices/gesture";
import { formatDb } from "@/features/devices/paramScale";
import { midiTarget } from "@/features/midi-learn/targets";
import { dbToFader, faderToDb } from "@/features/mixer/routing";
import { setDragCursor } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { groupVolumes, type VolumeSnapshot } from "./groupVolume";
import { useArrangementUi } from "./uiStore";

/**
 * Compact horizontal volume fader in a track header (the mixer view is gone). Drag left /
 * right (Shift: fine) as one undo step; arrows step it; double-click resets to 0 dB. The
 * dB value shows while hovering or dragging. When this track is part of a multi-track
 * selection, dragging or stepping moves every selected track by the same dB amount (their
 * balance is kept, see `groupVolumes`).
 */
export function HeaderVolume({ track }: { track: Track }) {
  const sender = useGestureSender();
  const db = track.mixer.volume;
  const pos = dbToFader(db);
  const drag = useRef<{
    x: number;
    pos: number;
    width: number;
    group: VolumeSnapshot | null;
  } | null>(null);
  const [active, setActive] = useState(false);

  /** Volumes of the selected tracks when this track is part of a multi-track selection. */
  const selectionSnapshot = (): VolumeSnapshot | null => {
    const selected = useArrangementUi.getState().selectedTracks;
    const tracks = useProjectStore.getState().project?.tracks;
    if (!tracks || selected.size < 2 || !selected.has(track.id)) return null;
    return new Map(
      [...selected].flatMap((id) =>
        tracks[id] ? [[id, tracks[id].mixer.volume] as const] : [],
      ),
    );
  };

  const set = (n: number, group: VolumeSnapshot | null = null) => {
    const v = faderToDb(Math.min(1, Math.max(0, n)));
    if (!group) {
      if (v !== db)
        void sender.send(
          cmd("Mixer", { type: "SetVolume", track: track.id, volume: v }),
        );
      return;
    }
    const tracks = useProjectStore.getState().project?.tracks;
    for (const [id, volume] of groupVolumes(group, track.id, v)) {
      if (tracks?.[id] && tracks[id].mixer.volume !== volume)
        void sender.send(
          cmd("Mixer", { type: "SetVolume", track: id, volume }),
        );
    }
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.stopPropagation(); // not a header drag / selection
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    e.currentTarget.focus({ preventScroll: true });
    drag.current = {
      x: e.clientX,
      pos,
      width: Math.max(1, e.currentTarget.clientWidth),
      group: selectionSnapshot(),
    };
    setActive(true);
    setDragCursor("ew-resize");
    sender.begin();
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const k = e.shiftKey ? 0.1 : 1;
    set(d.pos + ((e.clientX - d.x) / d.width) * k, d.group);
  };
  const end = () => {
    if (!drag.current) return;
    drag.current = null;
    setActive(false);
    setDragCursor(null);
    sender.end();
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 0.01 : 0.05;
    if (e.key === "ArrowRight" || e.key === "ArrowUp")
      set(pos + step, selectionSnapshot());
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown")
      set(pos - step, selectionSnapshot());
    else if (e.key === "Home") set(0);
    else if (e.key === "End") set(1);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div
      className={active ? "eth-arr-vol eth-arr-vol--active" : "eth-arr-vol"}
      role="slider"
      tabIndex={0}
      {...midiTarget({
        type: "Param",
        target: { type: "TrackVolume", track: track.id },
      })}
      aria-label={`${track.name} volume`}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={pos}
      aria-valuetext={formatDb(db)}
      title={`Volume ${formatDb(db)} (double-click: 0 dB)`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => {
        e.stopPropagation();
        set(dbToFader(0));
      }}
      onKeyDown={onKeyDown}
    >
      <span className="eth-arr-vol__track">
        <span
          className="eth-arr-vol__fill"
          style={{ transform: `scaleX(${pos})` }}
        />
        <span
          className="eth-arr-vol__thumb"
          style={{ left: `${pos * 100}%` }}
        />
      </span>
      <span className="eth-arr-vol__readout" aria-hidden>
        {formatDb(db)}
      </span>
    </div>
  );
}
