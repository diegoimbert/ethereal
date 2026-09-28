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
import { memo, useMemo, useRef, useState, type ComponentType, type PointerEvent, type ReactNode } from "react";
import { Headphones } from "lucide-react";
import type { Beats, Clip, CompRegion, TakeLane, Track } from "@/generated";
import { IconButton, openContextMenu, setDragCursor } from "@/kit";
import { useProjectStore } from "@/state";
import { pxToBeats, resolveGrid, snapToGrid, useTempoMap, type TempoMap, type TimelineViewport } from "@/timeline";
import { ClipView } from "@/features/arrangement/ClipView";
import { useArrangement } from "@/features/arrangement/context";
import { boundsOf } from "@/features/arrangement/editMath";
import { colorCss } from "@/features/arrangement/helpers";
import { beatsCss, widthCss } from "@/features/arrangement/laneGeometry";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { laneMenu, pickHere, renameLane, swipeComp, toggleAudition } from "./actions";
import { compOf, laneClipsOf, regionAt } from "./model";
import { TAKE_LANE_HEIGHT, useShownLanes } from "./height";
import { useCompingUi } from "./store";
import "./comping.css";

/** Pointer travel (px) before a press on a lane becomes a swipe. */
const SWIPE_SLOP_PX = 3;

export interface TakeLanesProps {
  track: Track;
  headerWidth: number;
  vp: TimelineViewport;
  visible: { start: Beats; end: Beats };
  /** Positions lane content at the live zoom/scroll (the arrangement's lane layer). */
  Layer: ComponentType<{ origin: Beats; children: ReactNode }>;
}

export const TakeLanes = memo(function TakeLanes({ track, headerWidth, vp, visible, Layer }: TakeLanesProps) {
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
        <TakeLaneRow key={lane.id} track={track} lane={lane} headerWidth={headerWidth} vp={vp} visible={visible} Layer={Layer} />
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

function TakeLaneRow({ track, lane, headerWidth, vp, visible, Layer }: RowProps) {
  const ctx = useArrangement();
  const { transport } = ctx;
  const tempo = useTempoMap();
  const clipsTable = useProjectStore((s) => s.project?.clips);
  const regionsTable = useProjectStore((s) => s.project?.comp_regions);
  const clips = useMemo(() => (clipsTable ? laneClipsOf({ clips: clipsTable }, lane.id) : []), [clipsTable, lane.id]);
  const regions = useMemo(() => (regionsTable ? compOf({ comp_regions: regionsTable }, track.id) : []), [regionsTable, track.id]);
  const mine = regions.filter((r) => r.lane === lane.id);
  const swipe = useCompingUi((s) => (s.swipe?.lane === lane.id ? s.swipe : null));
  const selected = useCompingUi((s) => (s.selected?.track === track.id ? s.selected.region : null));
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

  const menuAt = (clientX: number, el: HTMLElement) => {
    const at = beatAt(clientX, el, false);
    const clip = clips.find((c) => at >= c.start && at < c.start + c.length) ?? null;
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
        aria-label={`${lane.name} take`}
      >
        <span className="eth-takes__dot" aria-hidden title={hasComp ? "Part of the comp" : "Not in the comp"} />
        {renaming ? (
          <LaneNameInput
            name={lane.name}
            onDone={(name) => {
              setRenaming(false);
              if (name !== null) renameLane(transport, lane, name);
            }}
          />
        ) : (
          <span className="eth-takes__name" title={`${lane.name} (double-click to rename)`} onDoubleClick={() => setRenaming(true)}>
            {lane.name}
          </span>
        )}
        <IconButton
          className="eth-takes__audition"
          size="sm"
          tone="ghost"
          label={auditioning ? `Stop auditioning ${lane.name}` : `Audition ${lane.name}`}
          active={auditioning}
          icon={<Headphones />}
          onClick={() => toggleAudition(transport, lane)}
        />
      </div>
      <div
        className="eth-takes__lane"
        data-lane-take={lane.id}
        onPointerDown={onPointerDown}
        onContextMenu={(e) => openContextMenu(e, menuAt(e.clientX, e.currentTarget))}
      >
        <Layer origin={vp.scrollBeats}>
          <TakeClips clips={clips} color={lane.color ?? track.color} vp={vp} visible={visible} tempo={tempo} />
          {mine.map((r) => (
            <RegionMark key={r.id} region={r} origin={vp.scrollBeats} selected={selected === r.id} />
          ))}
          {swipe && (
            <div
              className="eth-takes__swipe"
              data-testid="swipe-preview"
              style={{ left: beatsCss(swipe.start - vp.scrollBeats), width: widthCss(swipe.end - swipe.start) }}
            />
          )}
        </Layer>
      </div>
    </div>
  );
}

/** The take's clips, drawn like arrangement clips but inert (the lane handles the pointer). */
const TakeClips = memo(function TakeClips({
  clips,
  color,
  vp,
  visible,
  tempo,
}: {
  clips: Clip[];
  color: number;
  vp: TimelineViewport;
  visible: { start: Beats; end: Beats };
  tempo: TempoMap;
}) {
  return (
    <div className="eth-takes__clips">
      {clips.map((c) =>
        c.start + c.length < visible.start || c.start > visible.end ? null : (
          <ClipView key={c.id} clip={c} bounds={boundsOf(c)} trackColor={color} vp={vp} visible={visible} tempo={tempo} />
        ),
      )}
    </div>
  );
});

function RegionMark({ region, origin, selected }: { region: CompRegion; origin: Beats; selected: boolean }) {
  return (
    <div
      className={clsx("eth-takes__region", selected && "eth-takes__region--selected")}
      data-region={region.id}
      style={{ left: beatsCss(region.start - origin), width: widthCss(region.end - region.start) }}
    />
  );
}

/** Inline rename field: Enter or blur commits, Escape cancels (`onDone(null)`). */
function LaneNameInput({ name, onDone }: { name: string; onDone: (name: string | null) => void }) {
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
      aria-label="Take name"
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
