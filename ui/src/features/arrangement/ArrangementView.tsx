import "./arrangement.css";
import { openContextMenu, setDragCursor } from "@/kit";
import { AddTrackRow } from "./newTrack";
import { useContext, useEffect, useMemo, useRef, type DragEvent, type KeyboardEvent, type PointerEvent } from "react";
import type { Beats, TrackId } from "@/generated";
import { useEditorStore, useProjectStore, useSelectionStore, useTracksOrdered } from "@/state";
import {
  gridLines,
  itemSelection,
  marqueeHits,
  PlayheadLine,
  pxToBeats,
  resolveGrid,
  Ruler,
  selectModeFromEvent,
  snapToGrid,
  useMarquee,
  useMiddleButtonPan,
  useTempoMap,
  useTimelineView,
  useTimelineWheel,
  useViewport,
  visibleRange,
} from "@/timeline";
import { useAutomationSlotHeight } from "@/features/automation";
import { PresenceLayer } from "@/features/collab/presence";
import { groupShortcut, groupTracks, ungroupSelected, UngroupConfirmDialog } from "@/features/groups";
import { TransportContext, useTransport, useTransportEvent } from "@/transport";
import { actionForKey, bindSingleSelection, locateIfStopped, newTrackMenu, runClipAction, selectTrackEntity } from "./actions";
import { dropBrowserMedia, hasBrowserDrag, readBrowserDrag } from "./browserDrop";
import { ArrangementContext, type ArrangementContextValue } from "./context";
import { clipRects, DROP_AREA_HEIGHT, HEADER_WIDTH, layoutRows, rowIndexAt, rowsHeight, type Row } from "./layout";
import { useLaneAnimation } from "./laneAnimation";
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
  // While automation lanes open/close: laid out once, animated by `useLaneAnimation`.
  const automationHeight = useAutomationSlotHeight();
  const draftTrack = useArrangementUi((s) => s.draftTrack);
  // The master track is pinned below the scrolling tracks (its own footer, at y 0); it is
  // laid out last, so the other rows keep their positions.
  const { rows, masterRow } = useMemo(() => {
    const all = layoutRows(tracks, folded, automationHeight, (id) => heights.get(id) ?? defaultHeight, draftTrack);
    const master = all.find((r) => r.track.kind === "Master");
    return { rows: all.filter((r) => r !== master), masterRow: master ? { ...master, y: 0 } : null };
  }, [tracks, folded, automationHeight, heights, defaultHeight, draftTrack]);
  const rowsRef = useRef<ReadonlyArray<Row>>(rows);
  useEffect(() => {
    rowsRef.current = rows;
  }, [rows]);

  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  useLaneAnimation(contentRef, rows, rowsRef);
  const onVerticalZoom = useTrackHeightZoom(scrollRef, rows);

  // A new draft track scrolls into view.
  useEffect(() => {
    if (!draftTrack) return;
    const row = rows.find((r) => r.draft);
    const el = scrollRef.current;
    if (!row || !el) return;
    if (row.y < el.scrollTop) el.scrollTop = row.y;
    else if (row.y + row.height > el.scrollTop + el.clientHeight) el.scrollTop = row.y + row.height - el.clientHeight;
    // Only when the draft appears or moves, not on every layout change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftTrack]);
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  useTimelineWheel(scrollRef, view, { smoothScrollY: true, onVerticalZoom, originPx: () => useArrangementUi.getState().headerWidth });
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

  // groups-buses: Cmd+G groups the selected tracks, Cmd+Shift+G ungroups. On the document,
  // so it still works after a menu or a click elsewhere took the focus (not in text fields
  // or dialogs).
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      const grouping = groupShortcut(e);
      if (!grouping || e.defaultPrevented || isTextEntry(e.target)) return;
      const active = document.activeElement;
      const root = rootRef.current;
      if (active && active !== document.body && !root?.contains(active)) return;
      e.preventDefault();
      const ui = useArrangementUi.getState();
      const fallback = useSelectionStore.getState().selectedTrack;
      const selected = ui.selectedTracks.size > 0 ? [...ui.selectedTracks] : fallback ? [fallback] : [];
      if (grouping === "ungroup") void ungroupSelected(transport, selected);
      else void groupTracks(transport, selected).then((g) => g && selectTrackEntity(g));
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [transport]);

  const ctx = useMemo<ArrangementContextValue>(
    () => ({ transport, peaks, contentRef, rowsRef, focus: () => rootRef.current?.focus({ preventScroll: true }) }),
    [transport, peaks],
  );

  const marquee = useMarquee({
    kind: "clip",
    hitTest: (rect) => {
      const project = useProjectStore.getState().project;
      const hw = useArrangementUi.getState().headerWidth;
      const lanes = { ...rect, x0: Math.max(rect.x0, hw), x1: Math.max(rect.x1, hw) };
      return project ? marqueeHits(lanes, clipRects(rowsRef.current, Object.values(project.clips), view.getState(), hw)) : [];
    },
    onClick: (p, ev) => {
      useArrangementUi.getState().setTrackFocus(null);
      const row = rowsRef.current[rowIndexAt(rowsRef.current, p.y)];
      if (row && !row.draft) useSelectionStore.getState().selectTrack(row.track.id);
      // A click on a clip's body (which lets presses through to the lane) selects the clip,
      // like its title bar does, but moves the playhead to the click, not the clip start.
      const project = useProjectStore.getState().project;
      if (project) {
        const hw = useArrangementUi.getState().headerWidth;
        const hit = clipRects(rowsRef.current, Object.values(project.clips), view.getState(), hw).find(
          ({ rect: r }) => p.x >= r.x0 && p.x < r.x1 && p.y >= r.y0 && p.y < r.y1,
        );
        if (hit) itemSelection.getState().select("clip", [hit.id], selectModeFromEvent(ev));
      }
      // A click on empty space also moves the playhead there (when stopped), snapped.
      const hw = useArrangementUi.getState().headerWidth;
      if (p.x >= hw) locateIfStopped(transport, snap(pxToBeats(p.x - hw, view.getState()), ev.altKey));
    },
  });

  /**
   * Pointer press in the tracks (capture, so clips' own handlers can't hide it): on a MIDI
   * clip it opens that clip in the piano roll; anywhere else it dismisses the piano roll
   * (the shell keeps it when pinned).
   */
  const onTracksPointerDownCapture = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const project = useProjectStore.getState().project;
    const box = contentRef.current?.getBoundingClientRect();
    if (!project || !box) return;
    const inTracks = contentRef.current!.contains(e.target as Node);
    const x = e.clientX - box.left;
    const y = e.clientY - box.top;
    const hw = useArrangementUi.getState().headerWidth;
    const hit = inTracks
      ? clipRects(rowsRef.current, Object.values(project.clips), view.getState(), hw).find(
          ({ rect: r }) => x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1,
        )
      : undefined;
    const clip = hit ? project.clips[hit.id] : undefined;
    if (clip?.content.type === "Midi") useEditorStore.getState().openClip(clip.id);
    else useEditorStore.getState().dismiss();
  };

  const tempo = useTempoMap();
  const snap = (beats: Beats, bypass: boolean): Beats => {
    if (bypass) return beats;
    const s = view.getState();
    return snapToGrid(beats, resolveGrid(grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats)), tempo);
  };

  const dropTarget = (e: DragEvent): DropTarget | "reject" => {
    const box = contentRef.current?.getBoundingClientRect();
    const x = e.clientX - (box?.left ?? 0) - useArrangementUi.getState().headerWidth;
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
      <div
        className="eth-arr"
        ref={rootRef}
        tabIndex={-1}
        onKeyDown={onKeyDown}
        data-feature="arrangement"
        style={{ ["--eth-arr-header-width" as string]: `${headerWidth}px` }}
      >
        <Toolbar />
        <div className="eth-arr__top">
          <div className="eth-arr__corner" style={{ width: headerWidth }} />
          <Ruler view={view} grid={grid} className="eth-arr__ruler" />
        </div>
        <div className="eth-arr__scroll" ref={scrollRef} onPointerDownCapture={onTracksPointerDownCapture}>
          <div
            className="eth-arr__content"
            ref={contentRef}
            style={{ height }}
            onPointerDown={(e) => {
              ctx.focus();
              // The selection box only starts over the lanes, not the header column.
              const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
              if (x >= useArrangementUi.getState().headerWidth) marquee.onPointerDown(e);
            }}
            onContextMenu={(e) => {
              // Empty space below the tracks (header column or lanes): add a track. Rows,
              // headers and clips open their own menus (and stop the event).
              const y = e.clientY - e.currentTarget.getBoundingClientRect().top;
              if (y >= rowsHeight(rowsRef.current)) openContextMenu(e, newTrackMenu(transport));
            }}
            onDragOver={onDragOver}
            onDragLeave={() => useArrangementUi.getState().setDropHint(null)}
            onDrop={onDrop}
            data-testid="arrangement-content"
          >
            <div className="eth-arr__backdrop" style={{ left: headerWidth }}>
              <GridLayer />
              <LoopLayer />
            </div>
            {rows.map((row) => (
              <TrackRow key={row.track.id} row={row} />
            ))}
            <div className="eth-arr__drop-area" style={{ height: DROP_AREA_HEIGHT }}>
              <AddTrackRow />
              <NewTrackDropHint />
            </div>
            <TrackDropLine />
            <div className="eth-arr__overlay" style={{ left: headerWidth }}>
              <PlayheadLine view={view} />
            </div>
            {m && (
              <div
                className="eth-arr__marquee"
                data-testid="marquee"
                style={{
                  left: Math.max(m.x0, headerWidth),
                  top: m.y0,
                  width: Math.max(0, m.x1 - Math.max(m.x0, headerWidth)),
                  height: m.y1 - m.y0,
                }}
              />
            )}
          </div>
        </div>
        {masterRow && (
          <div className="eth-arr__master" data-testid="arrangement-master" onPointerDownCapture={onTracksPointerDownCapture}>
            <div className="eth-arr__backdrop" style={{ left: headerWidth }}>
              <GridLayer />
            </div>
            <TrackRow row={masterRow} />
            <div className="eth-arr__overlay" style={{ left: headerWidth }}>
              <PlayheadLine view={view} />
            </div>
          </div>
        )}
        <HeaderColumnResizer />
        {/* presence-v2: peers' live pointers, pointer/viewport publishing, follow mode */}
        <PresenceLayer rootRef={rootRef} scrollRef={scrollRef} rows={rows} masterRow={masterRow} />
        <UngroupConfirmDialog />
      </div>
    </ArrangementContext.Provider>
  );
}

/**
 * Drag handle on the right edge of the track header column (full height): resizes it.
 * Double-click restores the default width.
 */
function HeaderColumnResizer() {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const w0 = headerWidth;
    setDragCursor("ew-resize");
    const move = (ev: globalThis.PointerEvent) => useArrangementUi.getState().setHeaderWidth(w0 + ev.clientX - x0);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      setDragCursor(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };
  return (
    <div
      className="eth-arr__header-resize"
      style={{ left: headerWidth - 3 }}
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize track headers"
      title="Drag to resize the track headers (double-click to reset)"
      onPointerDown={onPointerDown}
      onDoubleClick={() => useArrangementUi.getState().setHeaderWidth(HEADER_WIDTH)}
    />
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
