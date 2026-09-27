/**
 * Timeline ruler: bar/beat (or min:sec) ticks and labels, the loop brace and the playhead
 * marker, driven by a `TimelineViewStore`.
 *
 * Interactions (Ableton-like):
 * - press: locate the playhead there; drag: the playhead follows the pointer (both snapped
 *   to the grid; alt/option bypasses snapping). The view itself never moves;
 * - loop brace: drag the body to move it, the edges to resize (snapped); double-click
 *   toggles the loop; shift-drag on the ruler draws a new loop region;
 * - tempo map (shared touch, `features/tempo/RulerTempoMarkers`): tempo and time-signature
 *   markers (drag to move), right-click to add a change there;
 * - cmd/ctrl + wheel zooms, horizontal wheel scrolls (`useTimelineWheel`).
 * Loop edits are undoable document edits sent with a gesture (one undo step per drag).
 */

import { useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { BeatRange, Beats, Command } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { cmd, nextGestureId, useTransport } from "@/transport";
import { applyLoopDrag, loopFromPoints, type LoopHandle } from "./loop";
import { DEFAULT_GRID, resolveGrid, snapToGrid, type GridSetting } from "./grid";
import { useFollowPlayhead, usePlayheadPosition, type PlayheadMapping } from "./playhead";
import { rulerMarks, type RulerFormat } from "./rulerMarks";
import { useTempoMap, type TempoMap } from "./tempoMap";
// Shared touch (tempo-metronome): tempo-map markers and menu on the ruler.
import { openContextMenu } from "@/kit";
import { RulerTempoMarkers, rulerTempoMenu, useSortedTempoMap } from "@/features/tempo/RulerTempoMarkers";
import { useTimelineWheel } from "./useTimelineWheel";
import { beatsToPx, pxToBeats } from "./viewport";
import { useTimelineView, useViewport, type TimelineViewStore } from "./viewStore";
import "./timeline.css";

export interface RulerProps {
  view: TimelineViewStore;
  /** Bars/beats (default) or minutes:seconds. */
  format?: RulerFormat;
  /** Grid used to snap locate/loop edits (default: adaptive medium). */
  grid?: GridSetting;
  /** Show and edit the project loop brace (default true). */
  showLoop?: boolean;
  /** Override the tempo map (default: the project's). */
  tempo?: TempoMap;
  /** Show and edit the tempo map (tempo/signature markers, right-click to add; default:
   *  with the loop brace, on the project tempo map). */
  showTempo?: boolean;
  /** Maps song beats to this view's axis (clip-relative views). Default identity. */
  playheadMapping?: PlayheadMapping;
  /** Locate handler (default: `Transport::Locate`). Receives beats on this view's axis. */
  onLocate?: (beats: Beats) => void;
  /** Report the ruler's width to the view store (default true). */
  syncWidth?: boolean;
  height?: number;
  className?: string;
}

const EDGE_PX = 5;

export function Ruler({
  view,
  format = "bars",
  grid = DEFAULT_GRID,
  showLoop = true,
  tempo: tempoOverride,
  showTempo,
  playheadMapping,
  onLocate,
  syncWidth = true,
  height = 28,
  className,
}: RulerProps) {
  const transport = useTransport();
  const projectTempo = useTempoMap();
  const tempo = tempoOverride ?? projectTempo;
  const tempoEditable = (showTempo ?? (showLoop && tempoOverride === undefined)) && tempoOverride === undefined;
  const tempoTables = useSortedTempoMap();
  const vp = useViewport(view);
  const widthPx = useTimelineView(view, (s) => s.widthPx);
  const rootRef = useRef<HTMLDivElement>(null);
  const playheadRef = useRef<HTMLDivElement>(null);

  const loopEnabled = useProjectStore((s) => s.project?.settings.loop_enabled ?? false);
  const loopRegion = useProjectStore((s) => s.project?.settings.loop_region ?? null);
  /** Live region while dragging (before the engine echoes it back). */
  const [dragRegion, setDragRegion] = useState<BeatRange | null>(null);
  const region = dragRegion ?? loopRegion;

  usePlayheadPosition(playheadRef, view, playheadMapping);
  useFollowPlayhead(view, playheadMapping);
  useTimelineWheel(rootRef, view);

  // Width sync.
  useEffect(() => {
    const el = rootRef.current;
    if (!el || !syncWidth) return;
    const set = () => view.getState().setWidth(el.clientWidth);
    set();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(set);
    ro.observe(el);
    return () => ro.disconnect();
  }, [view, syncWidth]);

  const marks = useMemo(() => rulerMarks(tempo, vp, widthPx, format), [tempo, vp, widthPx, format]);

  const snap = (beats: Beats, bypass: boolean): Beats => {
    if (bypass) return beats;
    const st = view.getState();
    return snapToGrid(beats, resolveGrid(grid, st.pxPerBeat, tempo.signatureAt(st.scrollBeats)), tempo);
  };

  const send = (command: Command, gesture?: number) => {
    transport.send(command, gesture === undefined ? undefined : { gesture }).catch((err: unknown) => {
      console.warn("ruler command failed", err);
    });
  };

  const localX = (e: { clientX: number }) => e.clientX - (rootRef.current?.getBoundingClientRect().left ?? 0);

  /** Drag helper: window-level move/up listeners for one pointer gesture. */
  const track = (onMove: (e: PointerEvent) => void, onUp: (e: PointerEvent) => void) => {
    const up = (e: PointerEvent) => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      onUp(e);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const setLoop = (r: BeatRange, gesture: number) => {
    setDragRegion(r);
    send(cmd("Transport", { type: "SetLoopRegion", region: r }), gesture);
  };
  const endGesture = (gesture: number) => {
    send(cmd("Edit", { type: "EndGesture", gesture }));
  };

  const onLoopPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !loopRegion) return;
    e.stopPropagation();
    const box = e.currentTarget.getBoundingClientRect();
    const offset = e.clientX - box.left;
    const handle: LoopHandle = offset <= EDGE_PX ? "start" : offset >= box.width - EDGE_PX ? "end" : "move";
    const startX = localX(e);
    const base = loopRegion;
    const gesture = nextGestureId();
    let last: BeatRange = base;
    track(
      (ev) => {
        const delta = (localX(ev) - startX) / view.getState().pxPerBeat;
        const next = applyLoopDrag(base, handle, delta, (b) => snap(b, ev.altKey));
        if (next.start !== last.start || next.end !== last.end) {
          last = next;
          setLoop(next, gesture);
        }
      },
      () => {
        if (last !== base) endGesture(gesture);
        setDragRegion(null);
      },
    );
  };

  const onLoopDoubleClick = () => {
    send(cmd("Transport", { type: "SetLoopEnabled", enabled: !loopEnabled }));
  };

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const startX = localX(e);
    const startBeats = pxToBeats(startX, view.getState());

    if (e.shiftKey && showLoop) {
      const gesture = nextGestureId();
      let drawn: BeatRange | null = null;
      track(
        (ev) => {
          const b = pxToBeats(localX(ev), view.getState());
          const r = loopFromPoints(snap(startBeats, ev.altKey), snap(b, ev.altKey));
          if (!drawn || r.start !== drawn.start || r.end !== drawn.end) {
            drawn = r;
            setLoop(r, gesture);
          }
        },
        () => {
          if (drawn) endGesture(gesture);
          setDragRegion(null);
        },
      );
      return;
    }

    // Locate on press, then scrub: the playhead follows the pointer until release.
    let located: Beats | null = null;
    const locate = (x: number, bypass: boolean) => {
      const beats = Math.max(0, snap(pxToBeats(x, view.getState()), bypass));
      if (beats === located) return;
      located = beats;
      if (onLocate) onLocate(beats);
      else send(cmd("Transport", { type: "Locate", position: beats }));
    };
    locate(startX, e.altKey);
    track(
      (ev) => locate(localX(ev), ev.altKey),
      () => {},
    );
  };

  const loopLeft = region ? beatsToPx(region.start, vp) : 0;
  const loopWidth = region ? Math.max(2, (region.end - region.start) * vp.pxPerBeat) : 0;

  return (
    <div
      ref={rootRef}
      className={className ? `eth-ruler ${className}` : "eth-ruler"}
      style={{ height }}
      onPointerDown={onPointerDown}
      onContextMenu={
        tempoEditable
          ? (e) =>
              openContextMenu(
                e,
                rulerTempoMenu(transport, tempo, tempoTables.points, tempoTables.signatures, snap(pxToBeats(localX(e), view.getState()), e.altKey)),
              )
          : undefined
      }
      data-testid="ruler"
      role="slider"
      aria-label="Timeline ruler"
      aria-valuemin={0}
      aria-valuenow={Math.round(vp.scrollBeats * 1000) / 1000}
    >
      {marks.ticks.map((t, i) => (
        <div
          key={`t${i}`}
          className={t.major ? "eth-ruler__tick eth-ruler__tick--major" : "eth-ruler__tick"}
          style={{ transform: `translateX(${Math.round(t.x)}px)` }}
        />
      ))}
      {marks.labels.map((l, i) => (
        <span key={`l${i}`} className="eth-ruler__label" style={{ transform: `translateX(${Math.round(l.x) + 3}px)` }}>
          {l.text}
        </span>
      ))}
      {showLoop && region && (
        <div
          className={loopEnabled ? "eth-ruler__loop" : "eth-ruler__loop eth-ruler__loop--off"}
          style={{ transform: `translateX(${Math.round(loopLeft)}px)`, width: loopWidth }}
          onPointerDown={onLoopPointerDown}
          onDoubleClick={onLoopDoubleClick}
          data-testid="ruler-loop"
          title="Loop (drag to move, edges to resize, double-click to toggle)"
        />
      )}
      {tempoEditable && <RulerTempoMarkers vp={vp} widthPx={widthPx} snap={snap} />}
      <div ref={playheadRef} className="eth-ruler__playhead" data-testid="ruler-playhead" aria-hidden />
    </div>
  );
}
