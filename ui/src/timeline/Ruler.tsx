/**
 * Timeline ruler: bar/beat (or min:sec) ticks and labels, the loop brace and the playhead
 * marker, driven by a `TimelineViewStore`.
 *
 * Interactions (Ableton-like):
 * - press: locate the playhead there; drag: the playhead follows the pointer (both snapped
 *   to the grid; alt/option bypasses snapping). The view itself never moves;
 * - loop brace: drag the body to move it, the edges to resize (snapped); double-click
 *   toggles the loop; shift-drag on the ruler draws a new loop region;
 * - tempo map (with a `tempoEditor`, e.g. `features/tempo`'s `TempoRuler`): tempo and
 *   time-signature markers (drag to move), right-click to add a change there;
 * - cmd/ctrl + wheel zooms, horizontal wheel scrolls (`useTimelineWheel`).
 * Loop edits are undoable document edits sent with a gesture (one undo step per drag).
 */

import { useEffect, useRef, type ComponentType, type PointerEvent as ReactPointerEvent } from "react";
import type { BeatRange, Beats, Command } from "@/generated";
import { cmd, nextGestureId, useTransport, type EngineTransport } from "@/transport";
import { loopFromPoints } from "./loop";
import { LoopBrace } from "./LoopBrace";
import { DEFAULT_GRID, resolveGrid, snapToGrid, type GridSetting, type GridStep } from "./grid";
import { useFollowPlayhead, usePlayheadPosition, type PlayheadMapping } from "./playhead";
import { rulerMarks, type RulerFormat, type RulerMarks } from "./rulerMarks";
import { useTempoMap, type TempoMap } from "./tempoMap";
import { openContextMenu, useThemeColor, type ContextMenuEntry } from "@/kit";
import { useT } from "@/i18n";
import { font, fontSize, size } from "@/theme";
import { BeatCanvas, type BeatWindow } from "./BeatCanvas";
import { textSprite } from "./textSprite";
import { useTimelineWheel } from "./useTimelineWheel";
import { pxToBeats, type TimelineViewport } from "./viewport";
import { useTimelineView, useViewport, type TimelineViewStore } from "./viewStore";
import "./timeline.css";
import { renderDpr } from "./renderDpr";

/**
 * The tempo-map editing a ruler can host (its markers and its context-menu entries),
 * provided by `features/tempo` so the timeline doesn't depend on that feature.
 */
export interface RulerTempoEditor {
  Markers: ComponentType<{
    vp: TimelineViewport;
    widthPx: number;
    snap: (beats: Beats, bypass: boolean) => Beats;
    gridStep: () => GridStep | null;
  }>;
  /** Entries at the (snapped) song beat `beats`; `sig` places a signature change. */
  menu: (transport: EngineTransport, tempo: TempoMap, beats: Beats, sig: { raw?: Beats; step?: GridStep | null; free?: boolean }) => ContextMenuEntry[];
}

export interface RulerProps {
  view: TimelineViewStore;
  /** Bars/beats (default) or minutes:seconds. */
  format?: RulerFormat;
  /** Grid used to snap locate/loop edits (default: adaptive medium). */
  grid?: GridSetting;
  /** Show and edit the project loop brace (default true). */
  showLoop?: boolean;
  /** Draw the brace on the ruler (default true; false: it is elsewhere, e.g. the arrangement's
   *  loop strip, and shift-drag on the ruler still draws a new loop). */
  loopBrace?: boolean;
  /** Override the tempo map (default: the project's). */
  tempo?: TempoMap;
  /** Show and edit the tempo map (tempo/signature markers, right-click to add; default:
   *  with the loop brace, on the project tempo map). Needs `tempoEditor`. */
  showTempo?: boolean;
  /** The tempo-map editing (none = the ruler only shows the tempo map). */
  tempoEditor?: RulerTempoEditor;
  /** Maps song beats to this view's axis (clip-relative views). Default identity. */
  playheadMapping?: PlayheadMapping;
  /** Locate handler (default: `Transport::Locate`). Receives beats on this view's axis. */
  onLocate?: (beats: Beats) => void;
  /** Report the ruler's width to the view store (default true). */
  syncWidth?: boolean;
  height?: number;
  className?: string;
  /** Extra context-menu entries at a (song) beat position, after the tempo menu. */
  menuItems?: (beats: Beats) => ContextMenuEntry[];
}

export function Ruler({
  view,
  format = "bars",
  grid = DEFAULT_GRID,
  showLoop = true,
  loopBrace = true,
  tempo: tempoOverride,
  showTempo,
  playheadMapping,
  onLocate,
  syncWidth = true,
  height = 28,
  className,
  menuItems,
  tempoEditor,
}: RulerProps) {
  const t = useT();
  const transport = useTransport();
  const projectTempo = useTempoMap();
  const tempo = tempoOverride ?? projectTempo;
  const tempoEditable = !!tempoEditor && (showTempo ?? (showLoop && tempoOverride === undefined)) && tempoOverride === undefined;
  const TempoMarkers = tempoEditable ? tempoEditor.Markers : null;
  const vp = useViewport(view);
  const widthPx = useTimelineView(view, (s) => s.widthPx);
  const rootRef = useRef<HTMLDivElement>(null);
  const playheadRef = useRef<HTMLDivElement>(null);

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

  const tickColor = useThemeColor("borderLight");
  const majorColor = useThemeColor("textDim");
  const drawRulerMarks = (ctx: CanvasRenderingContext2D | null, win: BeatWindow, canvas: HTMLCanvasElement) => {
    const marks = rulerMarks(tempo, { pxPerBeat: win.ppb, scrollBeats: win.from }, win.width, format);
    drawMarks(ctx, canvas, marks, win.height, tickColor, majorColor);
  };

  const gridStep = () => {
    const st = view.getState();
    return resolveGrid(grid, st.pxPerBeat, tempo.signatureAt(st.scrollBeats));
  };
  const snap = (beats: Beats, bypass: boolean): Beats => (bypass ? beats : snapToGrid(beats, gridStep(), tempo));

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
    send(cmd("Transport", { type: "SetLoopRegion", region: r }), gesture);
  };
  const endGesture = (gesture: number) => {
    send(cmd("Edit", { type: "EndGesture", gesture }));
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

  return (
    <div
      ref={rootRef}
      className={className ? `eth-ruler ${className}` : "eth-ruler"}
      style={{ height }}
      onPointerDown={onPointerDown}
      onContextMenu={
        tempoEditable || menuItems
          ? (e) => {
              const at = pxToBeats(localX(e), view.getState());
              const extra = menuItems?.(at) ?? [];
              const tempoMenu = tempoEditable
                ? tempoEditor.menu(transport, tempo, snap(at, e.altKey), {
                    raw: at,
                    step: gridStep(),
                    free: e.altKey,
                  })
                : [];
              openContextMenu(e, tempoMenu.length && extra.length ? [...tempoMenu, "separator", ...extra] : [...tempoMenu, ...extra]);
            }
          : undefined
      }
      data-testid="ruler"
      role="slider"
      aria-label={t("timeline.ruler.label")}
      aria-valuemin={0}
      aria-valuenow={Math.round(vp.scrollBeats * 1000) / 1000}
    >
      {/* Ticks and labels on one canvas: as elements they were restyled and repainted on
          every pan/zoom frame (hundreds of them), forcing a page-wide repaint. */}
      <BeatCanvas
        view={view}
        height={height}
        draw={drawRulerMarks}
        deps={[tempo, format, tickColor, majorColor]}
        className="eth-ruler__marks"
        testId="ruler-marks"
      />
      {showLoop && loopBrace && <LoopBrace view={view} snap={snap} />}
      {TempoMarkers && <TempoMarkers vp={vp} widthPx={widthPx} snap={snap} gridStep={gridStep} />}
      <div ref={playheadRef} className="eth-ruler__playhead" data-testid="ruler-playhead" aria-hidden />
    </div>
  );
}

/**
 * Ticks and labels of `marks` (x relative to the canvas) on the ruler canvas. Labels are also
 * published as `data-labels` (`[text, x]` pairs, x in canvas px) for tests.
 */
function drawMarks(
  ctx: CanvasRenderingContext2D | null,
  canvas: HTMLCanvasElement,
  marks: RulerMarks,
  height: number,
  tickColor: string,
  majorColor: string,
): void {
  canvas.dataset.labels = JSON.stringify(marks.labels.map((l) => [l.text, Math.round(l.x) + 3]));
  if (!ctx) return;
  const dpr = renderDpr();
  // Like the former `.eth-ruler__tick` elements: 1px, 25% (minor) / 55% (major) high.
  for (const t of marks.ticks) {
    const h = Math.round(height * (t.major ? 0.55 : 0.25));
    ctx.fillStyle = t.major ? majorColor : tickColor;
    ctx.fillRect(Math.round(t.x), height - h, 1, h);
  }
  const y = parseFloat(size.rulerLabelTop) + parseFloat(size.rulerLabelLineHeight) / 2;
  const labelFont = `${fontSize.xs} ${font.mono}`;
  const labelHeight = Math.ceil(parseFloat(size.rulerLabelLineHeight)) + 4;
  // Pre-rendered labels (bar numbers repeat while panning and zooming; see textSprite.ts).
  for (const l of marks.labels) {
    const sprite = textSprite(l.text, labelFont, majorColor, labelHeight, dpr);
    ctx.drawImage(sprite.canvas, Math.round(l.x) + 3, Math.round(y - sprite.height / 2), sprite.width, sprite.height);
  }
}
