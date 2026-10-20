/**
 * Take lanes under a track row (the owner's T1 choice: Ableton 11 / Logic-style take lanes
 * with swipe comping). One row per lane: a header (name, audition, context menu) and the
 * lane, which draws the take's clips (dimmed) with the comp regions that select this lane
 * highlighted. Dragging across a lane is the swipe: on release, that range of the track
 * plays from this take (`Take::SetComp`, one undo step). A click without a drag picks this
 * take for the comp region under the pointer (or the clip's range) and selects it; Up/Down
 * then step through the takes for that region.
 */

import clsx from "clsx";
import { memo, useMemo, useRef, useState, type MouseEvent, type PointerEvent } from "react";
import { Headphones } from "lucide-react";
import type { Beats, TakeLane, Track } from "@/generated";
import { IconButton, openContextMenu, setDragCursor } from "@/kit";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection, pxToBeats, resolveGrid, snapToGrid, useTempoMap } from "@/timeline";
import { useArrangement } from "@/features/arrangement/context";
import { colorCss } from "@/features/arrangement/helpers";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { laneMenu, pickHere, renameLane, swipeComp, toggleAudition } from "./actions";
import { compOf, laneClipsOf, regionAt } from "@/domain/comping";
import { TAKE_LANE_HEIGHT, useShownLanes } from "./height";
import { useCompingUi } from "./store";
import "./comping.css";
import { useT } from "@/i18n";

/** Pointer travel (px) before a press on a lane becomes a swipe. */
const SWIPE_SLOP_PX = 3;

export interface TakeLanesProps {
  track: Track;
  headerWidth: number;
}

/**
 * The take lanes' headers and pointer handling; the take clips, regions and swipe preview
 * are drawn by the arranger canvas (arrangement/gl/layers/comping.ts).
 */
export const TakeLanes = memo(function TakeLanes({ track, headerWidth }: TakeLanesProps) {
  const lanes = useShownLanes(track);
  const error = useCompingUi((s) => (s.error?.track === track.id ? s.error.message : null));
  if (lanes.length === 0) return null;
  return (
    <div
      className="eth-takes"
      data-takes={track.id}
      style={{ ["--eth-track-color" as string]: colorCss(track.color) }}
      // Keep lane gestures away from the arrangement's marquee.
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
    >
      {lanes.map((lane) => (
        <TakeLaneRow key={lane.id} track={track} lane={lane} headerWidth={headerWidth} />
      ))}
      {error && (
        <div className="eth-takes__error" role="alert" style={{ left: headerWidth }}>
          {error}
        </div>
      )}
    </div>
  );
});

interface RowProps extends Omit<TakeLanesProps, "track"> {
  track: Track;
  lane: TakeLane;
}

function TakeLaneRow({ track, lane, headerWidth }: RowProps) {
  const t = useT();
  const ctx = useArrangement();
  const { transport } = ctx;
  const tempo = useTempoMap();
  const clipsTable = useProjectStore((s) => s.project?.clips);
  const regionsTable = useProjectStore((s) => s.project?.comp_regions);
  const clips = useMemo(() => (clipsTable ? laneClipsOf({ clips: clipsTable }, lane.id) : []), [clipsTable, lane.id]);
  const regions = useMemo(() => (regionsTable ? compOf({ comp_regions: regionsTable }, track.id) : []), [regionsTable, track.id]);
  const mine = regions.filter((r) => r.lane === lane.id);
  const auditioning = useCompingUi((s) => s.audition.get(track.id) === lane.id);
  const [renaming, setRenaming] = useState(false);
  const color = colorCss(lane.color ?? track.color);

  const beatAt = (clientX: number, el: HTMLElement, snap: boolean): Beats => {
    const s = arrangementView.getState();
    const x = clientX - el.getBoundingClientRect().left;
    const raw = Math.max(0, pxToBeats(x, s));
    if (!snap) return raw;
    const step = resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
    return Math.max(0, snapToGrid(raw, step, tempo));
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget;
    const x0 = e.clientX;
    const anchor = beatAt(x0, el, !e.altKey);
    const rawAt = beatAt(x0, el, false);
    let dragging = false;
    const ui = useCompingUi.getState();
    const move = (ev: globalThis.PointerEvent) => {
      if (!dragging && Math.abs(ev.clientX - x0) < SWIPE_SLOP_PX) return;
      if (!dragging) setDragCursor("ew-resize");
      dragging = true;
      const b = beatAt(ev.clientX, el, !ev.altKey);
      ui.setSwipe({ track: track.id, lane: lane.id, start: Math.min(anchor, b), end: Math.max(anchor, b) });
    };
    const done = (ev: globalThis.PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      window.removeEventListener("pointercancel", cancel);
      setDragCursor(null);
      const s = useCompingUi.getState().swipe;
      useCompingUi.getState().setSwipe(null);
      if (!dragging) {
        pickHere(transport, track.id, lane.id, rawAt);
        return;
      }
      void ev;
      if (s && s.end - s.start > 1e-6) {
        void swipeComp(transport, track.id, lane.id, s.start, s.end).then((id) => {
          if (id) useCompingUi.getState().select({ track: track.id, region: id });
        });
      }
    };
    const cancel = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      window.removeEventListener("pointercancel", cancel);
      setDragCursor(null);
      useCompingUi.getState().setSwipe(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done);
    window.addEventListener("pointercancel", cancel);
  };

  const clipAt = (clientX: number, el: HTMLElement) => {
    const at = beatAt(clientX, el, false);
    return clips.find((c) => at >= c.start && at < c.start + c.length) ?? null;
  };

  /**
   * A double-click on a take's clip selects it and opens it in the editor, expanding the
   * collapsed drawer and inspector, like a double-click on an arrangement clip.
   */
  const onDoubleClick = (e: MouseEvent<HTMLDivElement>) => {
    const clip = clipAt(e.clientX, e.currentTarget);
    if (!clip) return;
    itemSelection.getState().select("clip", [clip.id]);
    useEditorStore.getState().revealClip(clip.id);
  };

  const menuAt = (clientX: number, el: HTMLElement) => {
    const at = beatAt(clientX, el, false);
    const clip = clipAt(clientX, el);
    return laneMenu(transport, lane, { region: regionAt(regions, at), clip, rename: () => setRenaming(true) });
  };

  const hasComp = mine.length > 0;
  return (
    <div
      className={clsx("eth-takes__row", hasComp && "eth-takes__row--in-comp", auditioning && "eth-takes__row--audition")}
      style={{ height: TAKE_LANE_HEIGHT, ["--eth-take-color" as string]: color }}
      data-take-lane={lane.id}
    >
      <div
        className="eth-takes__header"
        style={{ width: headerWidth }}
        onContextMenu={(e) => openContextMenu(e, laneMenu(transport, lane, { region: null, clip: null, rename: () => setRenaming(true) }))}
        role="group"
        aria-label={t("comping.lane.label", { name: lane.name })}
      >
        <span className="eth-takes__dot" aria-hidden title={hasComp ? t("comping.lane.in_comp") : t("comping.lane.not_in_comp")} />
        {renaming ? (
          <LaneNameInput
            name={lane.name}
            onDone={(name) => {
              setRenaming(false);
              if (name !== null) renameLane(transport, lane, name);
            }}
          />
        ) : (
          <span className="eth-takes__name eth-user-text" title={t("comping.lane.name_title", { name: lane.name })} onDoubleClick={() => setRenaming(true)}>
            {lane.name}
          </span>
        )}
        <IconButton
          className="eth-takes__audition"
          size="sm"
          tone="ghost"
          label={auditioning ? t("comping.lane.stop_audition", { name: lane.name }) : t("comping.lane.audition", { name: lane.name })}
          active={auditioning}
          icon={<Headphones />}
          onClick={() => toggleAudition(transport, lane)}
        />
      </div>
      <div
        className="eth-takes__lane"
        data-lane-take={lane.id}
        onPointerDown={onPointerDown}
        onDoubleClick={onDoubleClick}
        onContextMenu={(e) => openContextMenu(e, menuAt(e.clientX, e.currentTarget))}
      />
    </div>
  );
}

/** Inline rename field: Enter or blur commits, Escape cancels (`onDone(null)`). */
function LaneNameInput({ name, onDone }: { name: string; onDone: (name: string | null) => void }) {
  const t = useT();
  const [value, setValue] = useState(name);
  const done = useRef(false);
  const finish = (v: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(v);
  };
  return (
    <input
      className="eth-takes__rename"
      aria-label={t("comping.lane.rename")}
      value={value}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onChange={(e) => setValue(e.target.value)}
      onPointerDown={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(value);
        else if (e.key === "Escape") finish(null);
      }}
      onBlur={() => finish(value)}
    />
  );
}
