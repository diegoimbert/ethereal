import "./arrangement.css";
import { useContext, useEffect, useMemo, useRef, type DragEvent, type KeyboardEvent } from "react";
import type { Beats, TrackId } from "@/generated";
import { useProjectStore, useSelectionStore, useTracksOrdered } from "@/state";
import {
  gridLines,
  marqueeHits,
  PlayheadLine,
  pxToBeats,
  resolveGrid,
  Ruler,
  snapToGrid,
  useMarquee,
  useMiddleButtonPan,
  useTempoMap,
  useTimelineView,
  useTimelineWheel,
  useViewport,
  visibleRange,
} from "@/timeline";
import { useAutomationHeight } from "@/features/automation";
import { TransportContext, useTransport, useTransportEvent } from "@/transport";
import { actionForKey, bindSingleSelection, locateIfStopped, runClipAction } from "./actions";
import { dropBrowserMedia, hasBrowserDrag, readBrowserDrag } from "./browserDrop";
import { ArrangementContext, type ArrangementContextValue } from "./context";
import { clipRects, DROP_AREA_HEIGHT, HEADER_WIDTH, layoutRows, rowIndexAt, rowsHeight, type Row } from "./layout";
import { PeakCache } from "./peaks";
import { Toolbar } from "./Toolbar";
import { ImportPlaceholder, TrackRow } from "./TrackRow";
import { arrangementView, useArrangementUi } from "./uiStore";
import { useFollowWithMargin } from "./useFollowWithMargin";
import { useTrackHeightZoom } from "./useTrackHeightZoom";

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

/** Where a browser drop at a client point lands: a track (or `null` = new track) and a beat. */
interface DropTarget {
  track: TrackId | null;
  at: Beats;
}

/**
 * Arrangement view: track headers and clip lanes under a shared ruler. Horizontal zoom and
 * scroll are virtual (`arrangementView`), vertical scrolling is native. Outside a
 * `<TransportProvider>` (the bare app shell in tests) it renders an empty placeholder.
 */
export function ArrangementView() {
  if (!useContext(TransportContext)) {
    return (
      <div className="eth-arr eth-arr--disconnected" data-feature="arrangement">
        No engine connection
      </div>
    );
  }
  return <ConnectedArrangementView />;
}

function ConnectedArrangementView() {
  const transport = useTransport();
  const view = arrangementView;
  const peaks = useMemo(() => new PeakCache(transport), [transport]);
  useTransportEvent((e) => {
    if (e.type === "Media" && e.event.type === "PeaksReady") peaks.invalidate(e.event.media);
  });

  const tracks = useTracksOrdered();
  const folded = useArrangementUi((s) => s.folded);
  const heights = useArrangementUi((s) => s.heights);
  const defaultHeight = useArrangementUi((s) => s.defaultHeight);
  const grid = useArrangementUi((s) => s.grid);
  const automationHeight = useAutomationHeight();
  const rows = useMemo(
    () => layoutRows(tracks, folded, automationHeight, (id) => heights.get(id) ?? defaultHeight),
    [tracks, folded, automationHeight, heights, defaultHeight],
  );
  const rowsRef = useRef<ReadonlyArray<Row>>(rows);
  useEffect(() => {
    rowsRef.current = rows;
  }, [rows]);

  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const onVerticalZoom = useTrackHeightZoom(scrollRef, rows);
  useTimelineWheel(scrollRef, view, { smoothScrollY: true, onVerticalZoom });
  useMiddleButtonPan(scrollRef, view);
  useFollowWithMargin(view);
  useEffect(() => bindSingleSelection(), []);

  // Copy/cut/paste also arrive as clipboard events: on macOS the app's Edit menu takes
  // cmd-C/X/V before the page sees the key (desktop app), and sends these instead.
  useEffect(() => {
    const onClipboard = (e: ClipboardEvent) => {
      const root = rootRef.current;
      if (!root || !root.contains(document.activeElement) || isTextEntry(document.activeElement)) return;
      e.preventDefault();
      void runClipAction(transport, e.type as "copy" | "cut" | "paste");
    };
    document.addEventListener("copy", onClipboard);
    document.addEventListener("cut", onClipboard);
    document.addEventListener("paste", onClipboard);
    return () => {
      document.removeEventListener("copy", onClipboard);
      document.removeEventListener("cut", onClipboard);
      document.removeEventListener("paste", onClipboard);
    };
  }, [transport]);

  const ctx = useMemo<ArrangementContextValue>(
    () => ({ transport, peaks, contentRef, rowsRef, focus: () => rootRef.current?.focus({ preventScroll: true }) }),
    [transport, peaks],
  );

  const marquee = useMarquee({
    kind: "clip",
    hitTest: (rect) => {
      const project = useProjectStore.getState().project;
      return project ? marqueeHits(rect, clipRects(rowsRef.current, Object.values(project.clips), view.getState())) : [];
    },
    onClick: (p, ev) => {
      useArrangementUi.getState().setTrackFocus(null);
      const row = rowsRef.current[rowIndexAt(rowsRef.current, p.y)];
      if (row) useSelectionStore.getState().selectTrack(row.track.id);
      // A click on empty space also moves the playhead there (when stopped), snapped.
      if (p.x >= HEADER_WIDTH) locateIfStopped(transport, snap(pxToBeats(p.x - HEADER_WIDTH, view.getState()), ev.altKey));
    },
  });

  const tempo = useTempoMap();
  const snap = (beats: Beats, bypass: boolean): Beats => {
    if (bypass) return beats;
    const s = view.getState();
    return snapToGrid(beats, resolveGrid(grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats)), tempo);
  };

  const dropTarget = (e: DragEvent): DropTarget | "reject" => {
    const box = contentRef.current?.getBoundingClientRect();
    const x = e.clientX - (box?.left ?? 0) - HEADER_WIDTH;
    const i = rowIndexAt(rowsRef.current, e.clientY - (box?.top ?? 0));
    const at = Math.max(0, snap(pxToBeats(Math.max(0, x), view.getState()), e.altKey));
    if (i >= rowsRef.current.length) return { track: null, at };
    const row = rowsRef.current[i];
    return row && row.track.kind === "Audio" ? { track: row.track.id, at } : "reject";
  };

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    if (!hasBrowserDrag(e.dataTransfer)) return;
    e.preventDefault();
    const t = dropTarget(e);
    e.dataTransfer.dropEffect = t === "reject" ? "none" : "copy";
    useArrangementUi.getState().setDropHint(t === "reject" ? null : t);
  };

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    const payload = readBrowserDrag(e.dataTransfer);
    useArrangementUi.getState().setDropHint(null);
    if (!payload) return;
    e.preventDefault();
    const t = dropTarget(e);
    if (t === "reject") return;
    // Import + (new track +) clip as one undo step; shows an "Importing…" placeholder.
    dropBrowserMedia(transport, payload, t).catch((err: unknown) => console.warn("browser drop failed", err));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (isTextEntry(e.target)) return;
    const action = actionForKey(e);
    if (!action) return;
    e.preventDefault();
    e.stopPropagation();
    void runClipAction(transport, action);
  };

  const height = rowsHeight(rows) + DROP_AREA_HEIGHT;
  const m = marquee.rect;

  return (
    <ArrangementContext.Provider value={ctx}>
      <div className="eth-arr" ref={rootRef} tabIndex={-1} onKeyDown={onKeyDown} data-feature="arrangement">
        <Toolbar />
        <div className="eth-arr__top">
          <div className="eth-arr__corner" style={{ width: HEADER_WIDTH }} />
          <Ruler view={view} grid={grid} className="eth-arr__ruler" />
        </div>
        <div className="eth-arr__scroll" ref={scrollRef}>
          <div
            className="eth-arr__content"
            ref={contentRef}
            style={{ height }}
            onPointerDown={(e) => {
              ctx.focus();
              marquee.onPointerDown(e);
            }}
            onDragOver={onDragOver}
            onDragLeave={() => useArrangementUi.getState().setDropHint(null)}
            onDrop={onDrop}
            data-testid="arrangement-content"
          >
            <div className="eth-arr__backdrop" style={{ left: HEADER_WIDTH }}>
              <GridLayer />
              <LoopLayer />
            </div>
            {rows.map((row) => (
              <TrackRow key={row.track.id} row={row} />
            ))}
            <div className="eth-arr__drop-area" style={{ height: DROP_AREA_HEIGHT }}>
              <NewTrackDropHint />
            </div>
            <TrackDropLine />
            <div className="eth-arr__overlay" style={{ left: HEADER_WIDTH }}>
              <PlayheadLine view={view} />
            </div>
            {m && (
              <div
                className="eth-arr__marquee"
                data-testid="marquee"
                style={{ left: m.x0, top: m.y0, width: m.x1 - m.x0, height: m.y1 - m.y0 }}
              />
            )}
          </div>
        </div>
      </div>
    </ArrangementContext.Provider>
  );
}

/** Where a dragged track header will land (a line between rows). */
function TrackDropLine() {
  const y = useArrangementUi((s) => (s.trackDrag && !s.trackDrag.into ? s.trackDrag.y : null));
  return y === null ? null : <div className="eth-arr__track-drop" style={{ top: y }} data-testid="track-drop-line" />;
}

/** Bar/beat lines behind the lanes. */
function GridLayer() {
  const vp = useViewport(arrangementView);
  const width = useTimelineView(arrangementView, (s) => s.widthPx);
  const grid = useArrangementUi((s) => s.grid);
  const tempo = useTempoMap();
  const lines = useMemo(() => {
    const step = resolveGrid(grid, vp.pxPerBeat, tempo.signatureAt(vp.scrollBeats));
    if (!step || width <= 0) return [];
    return gridLines(tempo, visibleRange(vp, width), step, 1000);
  }, [grid, vp, width, tempo]);
  return (
    <>
      {lines.map((l) => (
        <div
          key={l.beats}
          className={`eth-arr__gridline eth-arr__gridline--${l.level}`}
          style={{ transform: `translateX(${Math.round((l.beats - vp.scrollBeats) * vp.pxPerBeat)}px)` }}
        />
      ))}
    </>
  );
}

/** The project loop region, shaded across the lanes when the loop is on. */
function LoopLayer() {
  const vp = useViewport(arrangementView);
  const enabled = useProjectStore((s) => s.project?.settings.loop_enabled ?? false);
  const region = useProjectStore((s) => s.project?.settings.loop_region ?? null);
  if (!enabled || !region) return null;
  return (
    <div
      className="eth-arr__loop"
      data-testid="loop-region"
      style={{ left: (region.start - vp.scrollBeats) * vp.pxPerBeat, width: (region.end - region.start) * vp.pxPerBeat }}
    />
  );
}

function NewTrackDropHint() {
  const hint = useArrangementUi((s) => (s.dropHint && s.dropHint.track === null ? s.dropHint.at : null));
  const pending = useArrangementUi((s) => s.imports.find((i) => i.track === null));
  const empty = useProjectStore((s) => !s.project || Object.keys(s.project.clips).length === 0);
  if (hint === null && pending) return <ImportPlaceholder item={pending} />;
  if (hint === null) {
    return (
      <span className="eth-arr__drop-label">
        {empty
          ? "Drop a sample from the Browser, or double-click a MIDI track to create a clip"
          : "Drop audio files here to create a track"}
      </span>
    );
  }
  return <span className="eth-arr__drop-label eth-arr__drop-label--active">Create an audio track</span>;
}
