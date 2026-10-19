/**
 * Touch on the arrangement (#522, the design's §4), in the mobile UI only (the desktop, touch
 * laptops included, keeps its pointer handling):
 *
 *   - one-finger drag: pans, locked to an axis when it starts mostly along one (time through
 *     the view store, tracks through the native vertical scroll), with momentum on release;
 *     panning while playing stops following the playhead (the transport's Follow brings it back);
 *   - pinch: fingers side by side zoom time around their midpoint, one above the other zoom
 *     the track height; the midpoint's motion pans at the same time;
 *   - tap, double-tap, long-press: the desktop's click, double-click and context menu there
 *     (`touchSurface.ts`);
 *   - the ruler, faders and knobs keep their own pointer handling (a drag on the ruler scrubs);
 *   - track headers: a tap selects, a tap on the selected one opens its sheet (`TrackSheet`),
 *     long-press opens its menu, long-press then drag reorders it;
 *   - clips: a drag on a selected clip moves it (the whole selection) at once, on its wide trim
 *     handles trims it; an unselected clip is moved after a long-press (a plain drag pans, so
 *     scrolling a full arrangement never moves clips by accident). The desktop's own drag runs
 *     (`onClipPointerDown`): snapping, other tracks, new tracks below the last. Near the lanes'
 *     edges the view scrolls along (`edgeSpeed`); a second finger cancels;
 *   - selection without Shift or Ctrl: in select-more mode (`selectState.ts`) a tap on a clip
 *     toggles it and a drag on empty lane space draws a selection box; long-press then drag on
 *     empty lane space selects a time range, whose two handles (`rangeTouch.ts`) drag its ends.
 */
import { useEffect, type RefObject } from "react";
import { isReadOnly, useProjectStore } from "@/state";
import { itemSelection, resolveGrid, snapToGrid, TempoMap } from "@/timeline";
import { trackRangeOf, useTimeSelection } from "@/features/time-edits";
import type { ArrangementContextValue } from "@/features/arrangement/context";
import { onClipPointerDown } from "@/features/arrangement/clipDrag";
import { clipRects } from "@/features/arrangement/layout";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { clipGrabAt, edgeSpeed } from "./clipTouch";
import { useTouchDrag } from "./dragState";
import { startFling } from "./gestures/momentum";
import type { Point } from "./gestures/recognizer";
import { bindTouchSurface, replayClick, type TouchPolicy } from "./gestures/touchSurface";
import { openClipEditor } from "./editorState";
import { useMobileUi } from "./mode";
import { openSheet } from "./sheets";
import { useTouchSelect } from "./selectState";
import { dragRangeEdge, rangeBox, rangeHandleAt, type RangeEdge } from "./rangeTouch";

/** Controls that follow a pressed pointer themselves. */
const DIRECT = '[role="slider"], .eth-arr-vol, .eth-knob, [data-touch="direct"]';
const NATIVE = 'input, textarea, select, [contenteditable]:not([contenteditable="false"])';
/** A track header's own controls (buttons, fader, knob): a tap there is theirs. */
const HEADER_CONTROLS = 'button, [role="slider"], .eth-arr-vol, .eth-arr-pan';

export function touchPolicy(target: Element): TouchPolicy {
  if (target.closest(NATIVE)) return "native";
  if (target.closest(DIRECT)) return "direct";
  return "gesture";
}

/** Manual navigation while playing: stop following the playhead. */
function detachFollow(): void {
  const view = arrangementView.getState();
  if (view.followPlayhead && useProjectStore.getState().transport?.playing) view.setFollowPlayhead(false);
}

export function useTouchArranger(
  rootRef: RefObject<HTMLElement | null>,
  scrollRef: RefObject<HTMLElement | null>,
  onVerticalZoom: (factor: number, pointerY: number) => void,
  ctx: ArrangementContextValue,
): void {
  const mobile = useMobileUi((s) => s.enabled);
  useEffect(() => {
    const root = rootRef.current;
    const scroller = scrollRef.current;
    if (!mobile || !root || !scroller) return;
    let stopFling = () => undefined as void;
    const pan = (dx: number, dy: number) => {
      if (dx !== 0) arrangementView.getState().scrollByPx(-dx);
      if (dy !== 0) scroller.scrollTop -= dy;
    };

    /** Client point `p` is over the tracks' lanes (not the header column, the master or below). */
    const inLanes = (p: Point) => {
      const content = ctx.contentRef.current;
      if (!content) return false;
      const box = content.getBoundingClientRect();
      const rows = ctx.rowsRef.current;
      const last = rows[rows.length - 1];
      const y = p.y - box.top;
      return p.x - box.left >= useArrangementUi.getState().headerWidth && y >= 0 && y < (last ? last.y + last.height : 0);
    };

    /** The time range's handle at client `p`, if any. */
    const rangeHandle = (p: Point): RangeEdge | null => {
      const content = ctx.contentRef.current;
      const range = trackRangeOf(useTimeSelection.getState().selection);
      if (!content || !range) return null;
      const box = content.getBoundingClientRect();
      const r = rangeBox(range, ctx.rowsRef.current, arrangementView.getState());
      return r ? rangeHandleAt(r, p.x - box.left - useArrangementUi.getState().headerWidth, p.y - box.top) : null;
    };
    /** The range end being dragged (its handle). */
    let rangeEdge: RangeEdge | null = null;
    /** Drag the range end to client `p` (snapped to the arrangement grid). */
    const dragRange = (p: Point) => {
      const content = ctx.contentRef.current;
      const project = useProjectStore.getState().project;
      const sel = useTimeSelection.getState();
      const range = trackRangeOf(sel.selection);
      if (!rangeEdge || !content || !project || !range) return;
      const view = arrangementView.getState();
      const tempo = TempoMap.fromProject(project);
      const raw = view.scrollBeats + (p.x - content.getBoundingClientRect().left - useArrangementUi.getState().headerWidth) / view.pxPerBeat;
      const beats = snapToGrid(raw, resolveGrid(useArrangementUi.getState().grid, view.pxPerBeat, tempo.signatureAt(view.scrollBeats)), tempo);
      const next = dragRangeEdge(range, rangeEdge, beats);
      // Never down to nothing (that would be the insert marker, and the handles would go).
      if (next.end - next.start < 1e-6) return;
      // Dragged past the other end: this handle is now the other one.
      if (rangeEdge === "start" ? beats > range.end : beats < range.start) rangeEdge = rangeEdge === "start" ? "end" : "start";
      if (next.start !== range.start || next.end !== range.end) sel.setSelection({ ...range, ...next });
    };

    /** The clip a drag starting at client `p` grabs (move or trim), if it may. */
    const grabAt = (p: Point, afterLongPress: boolean) => {
      const content = ctx.contentRef.current;
      const project = useProjectStore.getState().project;
      if (!content || !project || isReadOnly()) return null;
      const box = content.getBoundingClientRect();
      const hw = useArrangementUi.getState().headerWidth;
      if (p.x - box.left < hw) return null;
      const boxes = clipRects(ctx.rowsRef.current, Object.values(project.clips), arrangementView.getState(), hw);
      const selected = itemSelection.getState().selected.clip;
      const hit = clipGrabAt(boxes, selected, p.x - box.left, p.y - box.top);
      if (!hit) return null;
      // Unselected clips only after a long-press: a plain drag over them pans.
      if (!selected.has(hit.clip) && !afterLongPress) return null;
      const clip = project.clips[hit.clip];
      return clip ? { clip, grab: hit.grab, selected: selected.has(hit.clip) } : null;
    };

    const unbind = bindTouchSurface(root, {
      policy: touchPolicy,
      onTouchStart: () => stopFling(),
      onTap: (g, target) => {
        const header = target.closest<HTMLElement>(".eth-arr-header[data-track]");
        // A double-tap on a clip: its editor (after the desktop's own double-click there).
        // Select-more mode: a tap on a clip adds it to the selection or takes it out (the
        // desktop's Ctrl-click).
        if (g.count === 1 && !header && useTouchSelect.getState().multi && grabAt(g, true)) {
          replayClick(target, g, 1, { ctrlKey: true });
          return true;
        }
        if (g.count === 2 && !header) {
          const hit = grabAt(g, true);
          if (hit) {
            replayClick(target, g, 2);
            openClipEditor(hit.clip);
            return true;
          }
        }
        // A tap on the selected track's header (not on one of its controls): its sheet (none in a
        // read-only view: everything on it edits).
        if (g.count !== 1 || isReadOnly() || !header || target.closest(HEADER_CONTROLS) || !header.classList.contains("eth-arr-header--selected")) return false;
        openSheet({ kind: "track", track: header.dataset.track! });
        return true;
      },
      onPanStart: (g, target) => {
        const header = target.closest(".eth-arr-header[data-track]");
        // An automation lane: long-press then drag is the desktop's drag there (a point moves,
        // else a time range); a plain drag pans. (Double-tap adds or deletes a point.)
        if (target.closest(".eth-auto-lane__svg")) {
          if (g.after === "longpress") return "replay";
          detachFollow();
          return "view";
        }
        // Long-press then drag on a track header: the desktop's header drag (reorder, into a group).
        if (header) {
          if (g.after === "longpress" && !target.closest(HEADER_CONTROLS)) return "replay";
        } else {
          const start = { x: g.startX, y: g.startY };
          // The time range's handles first (they reach outside the range).
          const edge = rangeHandle(start);
          if (edge) {
            rangeEdge = edge;
            return "handled";
          }
          const hit = grabAt(start, g.after === "longpress");
          if (hit) {
            const noop = () => undefined;
            onClipPointerDown(
              {
                button: 0,
                clientX: start.x,
                clientY: start.y,
                shiftKey: false,
                metaKey: false,
                ctrlKey: false,
                altKey: false,
                target,
                stopPropagation: noop,
                preventDefault: noop,
              },
              hit.clip,
              ctx,
              { mode: hit.grab, keepSelection: hit.selected },
            );
            useTouchDrag.setState({ point: { x: g.x, y: g.y }, grab: hit.grab });
            return "handled";
          }
          // Empty lane space: long-press then drag selects a time range (the desktop's Shift-drag);
          // in select-more mode a drag draws a selection box (the desktop's plain drag).
          if (inLanes(start)) {
            if (g.after === "longpress") return { mode: "replay", mods: { shiftKey: true } };
            if (useTouchSelect.getState().multi) return "replay";
          }
        }
        detachFollow();
        return "view";
      },
      onDragMove: (p) => {
        if (rangeEdge) dragRange(p);
        if (useTouchDrag.getState().grab) useTouchDrag.setState({ point: p });
      },
      onDragEnd: () => {
        rangeEdge = null;
        useTouchDrag.setState({ point: null, grab: null });
      },
      autoScroll: (p, dt) => {
        const box = scroller.getBoundingClientRect();
        const left = box.left + useArrangementUi.getState().headerWidth;
        const vx = edgeSpeed(p.x, left, box.right);
        const vy = edgeSpeed(p.y, box.top, box.bottom);
        if (vx === 0 && vy === 0) return { dx: 0, moved: false };
        const view = arrangementView.getState();
        const before = view.scrollBeats;
        if (vx !== 0) view.scrollByPx((vx * dt) / 1000);
        const dx = (arrangementView.getState().scrollBeats - before) * view.pxPerBeat;
        const top = scroller.scrollTop;
        if (vy !== 0) scroller.scrollTop += (vy * dt) / 1000;
        if (rangeEdge) dragRange(p);
        return { dx, moved: dx !== 0 || scroller.scrollTop !== top };
      },
      onPan: pan,
      onPanEnd: (vx, vy) => {
        stopFling = startFling(vx, vy, (dx, dy) => pan(dx, dy));
      },
      onPinchStart: detachFollow,
      onPinch: (g) => {
        if (g.axis === "x") {
          const lanesLeft = scroller.getBoundingClientRect().left + useArrangementUi.getState().headerWidth;
          arrangementView.getState().zoomBy(g.scale, g.x - lanesLeft);
        } else {
          onVerticalZoom(g.scale, g.y - scroller.getBoundingClientRect().top);
        }
        pan(g.dx, g.dy);
      },
    });
    return () => {
      stopFling();
      unbind();
    };
  }, [mobile, rootRef, scrollRef, onVerticalZoom, ctx]);
}
