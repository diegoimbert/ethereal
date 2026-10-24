import "./arrangement.css";
import { surfaceMenuEntries } from "@/ext/surfaces";
import { timelineMenuExtras } from "@/features/clip-editing/timelineMenu";
import { useContributions } from "@/ext";
import { AmbientBlobs, Button, openContextMenu, setDragCursor, useThemeColor } from "@/kit";
import { useProjectScreen } from "@/features/project/screenStore";
import { droppedFiles, hasOsFiles, importAudio, noteHover, openImportDialog, type DropPoint, type ImportSource } from "@/features/import";
import { isSampleDrag, readSampleDrop, relinkDrop, useMediaRefs } from "@/features/media-refs";
import { missingClipAt, type RelinkTarget } from "./missingDrop";
import { AddTrackRow } from "./newTrack";
import {
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  type DragEvent,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent,
} from "react";
import type { Beats, TrackId } from "@/generated";
import { useShellStore } from "@/state/shell";
import { isReadOnly, useEditorStore, useReadOnly, dismissOnRelease, showClipOnClick, useProjectStore, useSelectionStore, useTracksOrdered } from "@/state";
import {
  BeatCanvas,
  gridLines,
  itemSelection,
  marqueeHits,
  PlayheadLine,
  RenderFrozenLayer,
  pxToBeats,
  resolveGrid,
  selectModeFromEvent,
  snapToGrid,
  useMarquee,
  useMiddleButtonPan,
  useTempoMap,
  useTimelineWheel,
  type BeatWindow,
} from "@/timeline";
import { isGroupFoldAnimating, useAutomationSlotHeight, useLaneMotion } from "@/features/automation";
import { handleTakeKey, useTakesHeight } from "@/features/comping";
import {
  clipboardAction,
  runTimeAction,
  bindPlayFrom,
  isTimeTrack,
  placeInsertMarker,
  setPlayStart,
  TimeSelectionLayer,
  useArrangementTimeEdits,
} from "@/features/time-edits";
import { firstMatch, shortcutLabel } from "@/features/keymap";
import { groupTracks, ungroupSelected, UngroupConfirmDialog } from "@/features/groups";
import { TransportContext, useTransport, useTransportEvent } from "@/transport";
import { actionForKey, bindEditedClipToSelection, bindSingleSelection, newTrackMenu, runClipAction, selectAtLaneClick, selectTrackEntity } from "./actions";
import { dropBrowserMedia, hasBrowserDrag, hasBrowserMidiDrag, readBrowserDrag } from "./browserDrop";
import { acceptsDevices, dropDevicesOnTrack, isDeviceDrag } from "@/features/devices/trackDrop";
import { importMidiFile, isMidiName } from "@/features/midi-file/importMidi";
import { sourceName } from "@/features/import/sources";
import { ArrangementContext, type ArrangementContextValue } from "./context";
import { clipDragActive, MODIFIER_KEYS } from "./dragGuard";
import { clipRects, DROP_AREA_HEIGHT, HEADER_WIDTH, layoutRows, maxTrackDepth, rowIndexAt, rowsHeight, type Row } from "./layout";
import { useLaneAnimation } from "./laneAnimation";
import { PeakCache } from "./peaks";
import { Toolbar } from "./Toolbar";
import { ImportPlaceholder, NewTrackGhostRows, TrackRow } from "./TrackRow";
import { arrangementView, useArrangementUi } from "./uiStore";
import { useSmoothFollow } from "./useSmoothFollow";
import { GlClipLayer } from "./gl/GlClipLayer";
import { backendKind, useArrangerBackend } from "./gl/backendChoice";
import { glRenderer } from "./gl/active";
import { installArrangementTestApi } from "./testApi";
import { useTrackHeightZoom } from "./useTrackHeightZoom";
import { useZoomOutLimit } from "./useZoomOutLimit";
import { TempoRuler } from "@/features/tempo";
import { useTouchArranger } from "@/features/mobile/touchArranger";
import { ClipHandles } from "@/features/mobile/ClipHandles";
import { RangeHandles } from "@/features/mobile/RangeHandles";
import { useMobileUi } from "@/features/mobile/mode";
import { t, useT } from "@/i18n";

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
  const t = useT();
  const connected = !!useContext(TransportContext);
  const noProject = useProjectStore((s) => s.project === null);
  const launching = useProjectScreen((s) => s.launching);
  if (!connected) {
    return (
      <div className="eth-arr eth-arr--disconnected" data-feature="arrangement">
        {t("arrangement.view.no_engine")}
      </div>
    );
  }
  // Nothing is opened on launch; the project screen picks a project. Blank until
  // launch has decided (no "Open a project" flash before the project screen).
  if (noProject && launching) return <div className="eth-arr eth-arr--disconnected" data-feature="arrangement" />;
  if (noProject) {
    return (
      <div className="eth-arr eth-arr--disconnected eth-arr--no-project" data-feature="arrangement">
        <span>{t("arrangement.view.no_project")}</span>
        <Button tone="accent" onClick={() => useProjectScreen.getState().show()}>
          {t("arrangement.view.open_project")}
        </Button>
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
  // The header column widens to fit the deepest group nesting (each level shifts right).
  const depth = useMemo(() => maxTrackDepth(tracks), [tracks]);
  useEffect(() => useArrangementUi.getState().setHeaderDepth(depth), [depth]);
  const folded = useArrangementUi((s) => s.folded);
  const heights = useArrangementUi((s) => s.heights);
  const defaultHeight = useArrangementUi((s) => s.defaultHeight);
  const grid = useArrangementUi((s) => s.grid);
  // While automation lanes open/close: laid out once, animated by `useLaneAnimation`.
  const automationHeight = useAutomationSlotHeight();
  // v0.2 (`comping`): expanded take lanes under their tracks.
  const takesHeight = useTakesHeight();
  // The master track is pinned below the scrolling tracks (its own footer, at y 0); it is
  // laid out last, so the other rows keep their positions.
  // A group folding / unfolding keeps its rows laid out until the animation ends.
  const laneAnimating = useLaneMotion((s) => s.animating);
  const foldedForLayout = useMemo(() => {
    const out = new Set(folded);
    for (const g of folded) if (isGroupFoldAnimating(laneAnimating, g)) out.delete(g);
    return out;
  }, [folded, laneAnimating]);
  const { rows, masterRow } = useMemo(() => {
    const all = layoutRows(tracks, foldedForLayout, automationHeight, (id) => heights.get(id) ?? defaultHeight, takesHeight);
    const master = all.find((r) => r.track.kind === "Master");
    return {
      rows: all.filter((r) => r !== master),
      masterRow: master ? { ...master, y: 0 } : null,
    };
  }, [tracks, foldedForLayout, automationHeight, heights, defaultHeight, takesHeight]);
  const rowsRef = useRef<ReadonlyArray<Row>>(rows);
  useEffect(() => {
    rowsRef.current = rows;
  }, [rows]);

  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  // Extensions' layers over the tracks (`@/ext` "arrangement.layer").
  const layers = useContributions("arrangement.layer");
  const contentRef = useRef<HTMLDivElement>(null);
  useLaneAnimation(contentRef, rows, rowsRef);
  // The canvas reads its rows from `rowsRef`, but nothing else tells it they changed: folding
  // a group (UI state only: no project, selection or viewport change) left the lanes drawn
  // for the old layout until the next click. Redraw once the new rows are in `rowsRef`.
  useEffect(() => {
    glRenderer()?.requestRender(true);
  }, [rows]);
  const onVerticalZoom = useTrackHeightZoom(scrollRef, rows);
  useZoomOutLimit();

  const headerWidth = useArrangementUi((s) => s.headerWidth);
  useTimelineWheel(scrollRef, view, {
    smoothScrollY: true,
    onVerticalZoom,
    originPx: () => useArrangementUi.getState().headerWidth,
  });
  useMiddleButtonPan(scrollRef, view);
  useSmoothFollow(view);
  useEffect(() => bindSingleSelection(), []);
  useEffect(() => bindEditedClipToSelection(), []);
  // The insert marker placed while playing becomes the play start on the next stop.
  useEffect(() => bindPlayFrom(transport), [transport]);

  // Copy/cut/paste also arrive as clipboard events: on macOS the app's Edit menu takes
  // cmd-C/X/V before the page sees the key (desktop app), and sends these instead.
  useEffect(() => {
    const onClipboard = (e: ClipboardEvent) => {
      const root = rootRef.current;
      if (!root || !root.contains(document.activeElement) || isTextEntry(document.activeElement)) return;
      e.preventDefault();
      // section-edit: a time selection (or a time copy, for paste) takes it as a section.
      const kind = e.type as "copy" | "cut" | "paste";
      const time = clipboardAction(kind);
      if (time) void runTimeAction(transport, time);
      else void runClipAction(transport, kind);
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

  // Cmd+G groups the selected tracks, Cmd+Shift+G ungroups (keymap). On the document,
  // so it still works after a menu or a click elsewhere took the focus (not in text fields
  // or dialogs).
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      // keymap: `track.group` / `track.ungroup` (defaults Mod+G / Mod+Shift+G).
      // `clip.consolidate` (Mod+J) too, when nothing has focus (a click elsewhere): else
      // the browser takes Ctrl+J (Chrome opens its downloads). In the arrangement, its own
      // key handler already ran it (and prevented the default).
      const action = firstMatch(["track.group", "track.ungroup", "clip.consolidate"] as const, e);
      if (!action || e.defaultPrevented || isTextEntry(e.target)) return;
      const active = document.activeElement;
      const root = rootRef.current;
      if (active && active !== document.body && !root?.contains(active)) return;
      e.preventDefault();
      if (action === "clip.consolidate") {
        void runClipAction(transport, "consolidate");
        return;
      }
      const grouping = action === "track.ungroup" ? "ungroup" : "group";
      const ui = useArrangementUi.getState();
      const fallback = useSelectionStore.getState().selectedTrack;
      const selected = ui.selectedTracks.size > 0 ? [...ui.selectedTracks] : fallback ? [fallback] : [];
      if (grouping === "ungroup") void ungroupSelected(transport, selected);
      else void groupTracks(transport, selected).then((g) => g && selectTrackEntity(g));
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [transport]);

  // The arranger's backend: WebGL2, else the Canvas2D fallback (gl/backendChoice.ts). A
  // failure remounts the clip layer (a retry, then the fallback).
  const backend = useArrangerBackend((s) => backendKind(s));
  const attempt = useArrangerBackend((s) => s.attempt);
  const ctx = useMemo<ArrangementContextValue>(
    () => ({
      transport,
      peaks,
      contentRef,
      rowsRef,
      focus: () => rootRef.current?.focus({ preventScroll: true }),
    }),
    [transport, peaks],
  );
  // The mobile UI: pan, pinch, tap, long-press and clip drags on touch screens.
  useTouchArranger(rootRef, scrollRef, onVerticalZoom, ctx);
  useEffect(
    () =>
      installArrangementTestApi({
        content: () => contentRef.current,
        scroller: () => scrollRef.current,
        rows: () => rowsRef.current,
      }),
    [],
  );

  // time-edits: a drag selects clips; a Shift-drag makes the time section (live); shortcuts
  // and the section's menu.
  const timeEdits = useArrangementTimeEdits(transport, rowsRef);
  const marquee = useMarquee({
    kind: "clip",
    // The lanes start after the header column; dragging past their edges scrolls the timeline.
    timeline: { view, originPx: () => useArrangementUi.getState().headerWidth },
    // The whole drag, past the lanes' edges too: clips the edge scroll moved off screen (left
    // of the lanes, under the header column) stay selected.
    hitTest: (rect) => {
      const project = useProjectStore.getState().project;
      const hw = useArrangementUi.getState().headerWidth;
      return project ? marqueeHits(rect, clipRects(rowsRef.current, Object.values(project.clips), view.getState(), hw)) : [];
    },
    onEnd: timeEdits.onMarqueeEnd,
    section: {
      ...timeEdits.section,
      // A Shift-click with a time selection (or the insert marker) extends / retracts it to
      // the click; it still selects the track range like a Shift-click on the header.
      onShiftClick: (p, ev) =>
        timeEdits.shiftClick(p, () => {
          const row = rowsRef.current[rowIndexAt(rowsRef.current, p.y)];
          if (row) selectAtLaneClick(row.track.id, null, selectModeFromEvent({ shiftKey: true, metaKey: ev.metaKey, ctrlKey: ev.ctrlKey }));
        }),
    },
    onClick: (p, ev) => {
      timeEdits.onMarqueeClick();
      const row = rowsRef.current[rowIndexAt(rowsRef.current, p.y)];
      const track = row ? row.track.id : null;
      // A click on a clip's body (which lets presses through to the lane) selects the clip,
      // like its title bar does, but moves the playhead to the click, not the clip start.
      // On empty space it selects the track, like a click on its header (the inspector
      // shows it). Both renderers share this hit test (clip rects, not the canvas).
      const project = useProjectStore.getState().project;
      const hit = project
        ? clipRects(rowsRef.current, Object.values(project.clips), view.getState(), useArrangementUi.getState().headerWidth).find(
            ({ rect: r }) => p.x >= r.x0 && p.x < r.x1 && p.y >= r.y0 && p.y < r.y1,
          )
        : undefined;
      // Before the marker: selecting tracks clears the time selection.
      selectAtLaneClick(track, hit?.id ?? null, selectModeFromEvent(ev));
      // A click on empty space (or a clip body) places the insert marker there on that
      // track, snapped (Alt: free); it never moves a playing playhead (see time-edits
      // `marker.ts`: while stopped, Play then starts from the marker).
      // Rows without a timeline of their own (returns, master) just set the cursor.
      const hw = useArrangementUi.getState().headerWidth;
      if (p.x >= hw) {
        const at = snap(pxToBeats(p.x - hw, view.getState()), ev.altKey);
        if (row && isTimeTrack(row.track)) placeInsertMarker(transport, at, [row.track.id]);
        else setPlayStart(transport, at);
      }
    },
  });

  /**
   * Pointer press in the tracks (capture, so clips' own handlers can't hide it). On a clip it
   * shows that clip in the editor drawer and the inspector, each as a bar unless already
   * expanded (a click never expands a pane: a double-click expands the drawer). Anywhere else it collapses the drawer, and on the lanes'
   * empty space the inspector too (a track header expands it: TrackRow), unless it shows that
   * row's track (or a clip on it) already; pinned, each stays.
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
      ? clipRects(rowsRef.current, Object.values(project.clips), view.getState(), hw).find(({ rect: r }) => x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1)
      : undefined;
    const clip = hit ? project.clips[hit.id] : undefined;
    const shell = useShellStore.getState();
    if (!clip) {
      dismissOnRelease();
      // Empty space of the track the inspector already shows (expanded), or of the track of a
      // clip it shows: it stays open. (The click then selects that track: the inspector
      // switches to it, as the clip selection clears.)
      const row = rowsRef.current[rowIndexAt(rowsRef.current, y)];
      const shown = itemSelection.getState().selected.clip;
      const inspected = (id: TrackId) =>
        shown.size > 0 ? [...shown].some((c) => project.clips[c]?.track === id) : id === useArrangementUi.getState().trackFocus;
      const showing = !!row && inspected(row.track.id) && shell.right.open && !shell.right.collapsed;
      if (inTracks && x >= hw && !shell.right.pinned && !showing) shell.setCollapsed("right", true);
      return;
    }
    // The clip gets selected: an inspector not showing yet appears as its bar (a click never
    // expands a pane); one already showing stays as it is.
    if (!shell.right.open && !shell.right.pinned) shell.setCollapsed("right", true);
    const show = () => {
      const { bottom, setCollapsed } = useShellStore.getState();
      if (!bottom.open || bottom.collapsed) setCollapsed("bottom", true);
      useEditorStore.getState().openClip(clip.id);
    };
    if (clip.content.type === "Midi") show();
    // Audio: a click (no drag) shows it; a modifier press (multi-select) leaves the drawer as
    // it is.
    else if (!(e.shiftKey || e.metaKey || e.ctrlKey)) showClipOnClick(e, show);
  };

  const tempo = useTempoMap();
  const snap = (beats: Beats, bypass: boolean): Beats => {
    if (bypass) return beats;
    const s = view.getState();
    return snapToGrid(beats, resolveGrid(grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats)), tempo);
  };

  /**
   * A click on the pinned master row's lane (outside the scrolling rows' marquee): like a
   * click on another row without a timeline of its own, it selects the master and sets the
   * cursor there (snapped; Alt: free).
   */
  const onMasterLaneClick = (e: ReactMouseEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !masterRow || !(e.target as Element).closest?.(".eth-arr-lane")) return;
    const hw = useArrangementUi.getState().headerWidth;
    const x = e.clientX - e.currentTarget.getBoundingClientRect().left - hw;
    if (x < 0) return;
    selectAtLaneClick(masterRow.track.id, null, selectModeFromEvent(e));
    setPlayStart(transport, Math.max(0, snap(pxToBeats(x, view.getState()), e.altKey)));
  };

  /**
   * Where a drop of `kind` files lands: on a track of that kind (audio clips on an audio
   * track, a MIDI file on a MIDI track), or below the tracks (new tracks). Other rows refuse.
   */
  const dropBeat = (e: DropPoint): Beats => {
    const box = contentRef.current?.getBoundingClientRect();
    const x = e.clientX - (box?.left ?? 0) - useArrangementUi.getState().headerWidth;
    return Math.max(0, snap(pxToBeats(Math.max(0, x), view.getState()), e.altKey));
  };
  const dropTarget = (e: DropPoint, kind: "Audio" | "Midi" = "Audio"): DropTarget | "reject" => {
    const box = contentRef.current?.getBoundingClientRect();
    const i = rowIndexAt(rowsRef.current, e.clientY - (box?.top ?? 0));
    const at = dropBeat(e);
    if (i >= rowsRef.current.length) return { track: null, at };
    const row = rowsRef.current[i];
    return row && row.track.kind === kind ? { track: row.track.id, at } : "reject";
  };

  // `file-import`: OS files dropped on a lane (clips from the drop point, one after the
  // other) or below the tracks (a new audio track per file). On the desktop the shell
  // forwards the dropped paths instead of the drop: they land here via `noteHover`.
  // MIDI files among them become MIDI clips (`importMidiFile`): on a MIDI track, or
  // on new MIDI tracks below the tracks.
  const importAt = (sources: ImportSource[], point: DropPoint) => {
    useArrangementUi.getState().setDropHint(null);
    const midi = sources.filter((s) => isMidiName(sourceName(s)));
    const audio = sources.filter((s) => !midi.includes(s));
    // OS files' kinds are only known now: a MIDI file over another kind of track goes on new
    // MIDI tracks.
    const mt = dropTarget(point, "Midi");
    const midiTarget = mt === "reject" ? { track: null, at: dropBeat(point) } : mt;
    for (const s of midi) importMidiFile(transport, s, midiTarget).catch((err: unknown) => console.warn("MIDI file import failed", err));
    const t = dropTarget(point);
    if (t === "reject" || audio.length === 0) return;
    void importAudio(transport, audio, t);
  };

  /** The track row under a drag (`null`: below the tracks). */
  const rowTrackAt = (e: DropPoint) => {
    const box = contentRef.current?.getBoundingClientRect();
    return rowsRef.current[rowIndexAt(rowsRef.current, e.clientY - (box?.top ?? 0))]?.track ?? null;
  };
  // Device cards dragged from a chain onto a track (header or lane): moved there (copied
  // with ⌘ / Ctrl), `@/features/devices/trackDrop`.
  const onDeviceDragOver = (e: DragEvent<HTMLDivElement>) => {
    const track = rowTrackAt(e);
    const ok = track !== null && acceptsDevices(track);
    if (ok) e.preventDefault();
    e.dataTransfer.dropEffect = !ok ? "none" : e.metaKey || e.ctrlKey ? "copy" : "move";
    useArrangementUi.getState().setDeviceDrop(ok ? track.id : null);
  };

  /** The clip under a sample drag whose sample is missing (the drop relinks it). */
  const relinkTargetAt = (e: DropPoint): RelinkTarget | null => {
    const project = useProjectStore.getState().project;
    const box = contentRef.current?.getBoundingClientRect();
    const y = e.clientY - (box?.top ?? 0);
    const row = rowsRef.current[rowIndexAt(rowsRef.current, y)];
    // The main lane only (not its take lanes).
    if (!project || !row || row.track.kind !== "Audio" || y - row.y >= row.laneHeight - (row.takesHeight ?? 0)) return null;
    const x = e.clientX - (box?.left ?? 0) - useArrangementUi.getState().headerWidth;
    if (x < 0) return null;
    return missingClipAt(project.clips, row.track.id, pxToBeats(x, view.getState()), useMediaRefs.getState().missing);
  };

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    if (isReadOnly()) return;
    if (isDeviceDrag(e.dataTransfer.types)) return onDeviceDragOver(e);
    const files = hasOsFiles(e.dataTransfer);
    if (!files && !hasBrowserDrag(e.dataTransfer)) return;
    e.preventDefault();
    // A sample over a clip whose sample is missing: relinks it.
    const relink = isSampleDrag(e.dataTransfer) ? relinkTargetAt(e) : null;
    useArrangementUi.getState().setRelinkDrop(relink);
    if (relink) {
      e.dataTransfer.dropEffect = "copy";
      useArrangementUi.getState().setDropHint(null);
      if (files)
        noteHover(e, (sources) => {
          useArrangementUi.getState().setRelinkDrop(null);
          void relinkDrop(transport, relink.media, { kind: "files", sources });
        });
      return;
    }
    // A browser MIDI file goes on MIDI tracks; OS files (their kinds aren't known until the
    // drop) on audio or MIDI tracks.
    let t = dropTarget(e, hasBrowserMidiDrag(e.dataTransfer) ? "Midi" : "Audio");
    if (files && t === "reject") t = dropTarget(e, "Midi");
    e.dataTransfer.dropEffect = t === "reject" ? "none" : "copy";
    useArrangementUi.getState().setDropHint(t === "reject" ? null : t);
    if (files && t !== "reject") noteHover(e, importAt);
  };

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    if (isReadOnly()) return;
    if (isDeviceDrag(e.dataTransfer.types)) {
      useArrangementUi.getState().setDeviceDrop(null);
      const track = rowTrackAt(e);
      if (!track || !acceptsDevices(track)) return;
      e.preventDefault();
      void dropDevicesOnTrack(transport, e.dataTransfer, track.id, e.metaKey || e.ctrlKey);
      return;
    }
    useArrangementUi.getState().setRelinkDrop(null);
    const relink = relinkTargetAt(e);
    const sample = relink ? readSampleDrop(e.dataTransfer) : null;
    if (relink && sample) {
      e.preventDefault();
      useArrangementUi.getState().setDropHint(null);
      void relinkDrop(transport, relink.media, sample);
      return;
    }
    if (hasOsFiles(e.dataTransfer)) {
      e.preventDefault();
      importAt(droppedFiles(e.dataTransfer), e);
      return;
    }
    const payload = readBrowserDrag(e.dataTransfer);
    useArrangementUi.getState().setDropHint(null);
    if (!payload) return;
    e.preventDefault();
    if (payload.file_kind === "Midi") {
      const mt = dropTarget(e, "Midi");
      if (mt === "reject") return;
      importMidiFile(transport, { kind: "media", source: payload.source, name: payload.name }, mt).catch((err: unknown) =>
        console.warn("MIDI file drop failed", err),
      );
      return;
    }
    const t = dropTarget(e);
    if (t === "reject") return;
    // Import + (new track +) clip as one undo step; shows an "Importing…" placeholder.
    dropBrowserMedia(transport, payload, t).catch((err: unknown) => console.warn("browser drop failed", err));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (isTextEntry(e.target)) return;
    // A clip drag in progress: its clips aren't edited under it (Escape cancels the drag;
    // modifiers still switch copy / snapping).
    if (clipDragActive()) {
      if (!MODIFIER_KEYS.has(e.key)) {
        e.preventDefault();
        e.stopPropagation();
      }
      return;
    }
    if (timeEdits.onKeyDown(e)) return;
    // v0.2 (`comping`): Up/Down pick the previous/next take for the selected comp region.
    if (handleTakeKey(e, transport)) {
      e.preventDefault();
      e.stopPropagation();
      return;
    }
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
        data-keymap-scope="arrangement"
        data-feature="arrangement"
        style={{ ["--eth-arr-header-width" as string]: `${headerWidth}px` }}
      >
        {/* The modals' color blobs, very faintly, behind the whole arrangement. */}
        <AmbientBlobs className="eth-arr__blobs" strength={0.28} />
        <div className="eth-arr__top">
          {/* The empty corner above the track headers holds the arrangement's tools. */}
          <div className="eth-arr__corner" style={{ width: headerWidth }}>
            <Toolbar />
          </div>
          <TempoRuler
            // Its tempo and signature changes are in the marker lane, its loop brace on the
            // strip above it.
            markers={false}
            loopBrace={false}
            view={view}
            grid={grid}
            className="eth-arr__ruler"
            // After its tempo entries, the marker band's (one timeline menu on both).
            menuItems={(at) => timelineMenuExtras(transport, at)}
          />
        </div>
        <div className="eth-arr__scroll" ref={scrollRef} onPointerDownCapture={onTracksPointerDownCapture}>
          <div
            className="eth-arr__content"
            ref={contentRef}
            style={{ height }}
            onPointerDown={(e) => {
              ctx.focus();
              timeEdits.onPointerDown(e);
              // The selection box only starts over the lanes, not the header column.
              const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
              if (x >= useArrangementUi.getState().headerWidth) marquee.onPointerDown(e);
            }}
            onContextMenuCapture={timeEdits.onContextMenuCapture}
            onContextMenu={(e) => {
              // Empty space below the tracks (header column or lanes): add a track. Rows,
              // headers and clips open their own menus (and stop the event).
              const y = e.clientY - e.currentTarget.getBoundingClientRect().top;
              if (y >= rowsHeight(rowsRef.current)) {
                openContextMenu(e, [
                  ...newTrackMenu(transport),
                  "separator",
                  {
                    label: t("arrangement.view.import_audio"),
                    shortcut: shortcutLabel("file.import"),
                    onSelect: () => void openImportDialog(transport),
                  },
                  ...surfaceMenuEntries({ kind: "arranger" }, e),
                ]);
              }
            }}
            onDragOver={onDragOver}
            onDragLeave={(e) => {
              // Leaving the content (not moving between its children).
              if (e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget)) return;
              useArrangementUi.getState().setDropHint(null);
              useArrangementUi.getState().setRelinkDrop(null);
              useArrangementUi.getState().setDeviceDrop(null);
            }}
            onDrop={onDrop}
            data-testid="arrangement-content"
          >
            <div className="eth-arr__backdrop" style={{ left: headerWidth }}>
              <GridLayer />
              <LoopLayer />
            </div>
            <GlClipLayer key={`${backend}:${attempt}`} backend={backend} ctx={ctx} scrollRef={scrollRef} />
            {/* The header column's surface over the clips drawn under it: translucent (blurred)
                or, with translucency off, opaque. */}
            <div className="eth-arr__header-shade" style={{ width: headerWidth }} aria-hidden />
            {rows.map((row) => (
              <TrackRow key={row.track.id} row={row} />
            ))}
            <div className="eth-arr__drop-area" style={{ height: DROP_AREA_HEIGHT }}>
              <AddTrackRow />
              <NewTrackDropHint />
              <NewTrackGhostRows />
            </div>
            <TrackDropLine />
            <div className="eth-arr__overlay" style={{ left: headerWidth }}>
              <TimeSelectionLayer rows={rows} view={view} headerWidth={0} />
              {/* A render runs: past its rendered part (and before its start) is frozen. */}
              <RenderFrozenLayer view={view} />
              <PlayheadLine view={view} />
              {/* The mobile UI: trim handles of the selected clips, the time range's handles. */}
              <ClipHandles rows={rows} />
              <RangeHandles rows={rows} />
            </div>
            {m && (
              <div
                className="eth-arr__marquee"
                data-testid="marquee"
                // Past the lanes' left edge (under the header column) it goes on: no left border.
                data-cut={[marquee.cut, m.x0 < headerWidth && !marquee.cut.includes("left") ? "left" : ""].join(" ").trim() || undefined}
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
          // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- a pointer gesture on the lane (the transport's keys move the cursor)
          <div className="eth-arr__master" data-testid="arrangement-master" onPointerDownCapture={onTracksPointerDownCapture} onClick={onMasterLaneClick}>
            <div className="eth-arr__backdrop" style={{ left: headerWidth }}>
              <GridLayer />
            </div>
            <TrackRow row={masterRow} />
            <div className="eth-arr__overlay" style={{ left: headerWidth }}>
              <RenderFrozenLayer view={view} />
              <PlayheadLine view={view} />
            </div>
          </div>
        )}
        {/* Extensions' layers over the tracks (`@/ext` "arrangement.layer"). */}
        {layers.map((Layer, i) => (
          <Layer key={i} rootRef={rootRef} scrollRef={scrollRef} rows={rows} masterRow={masterRow} />
        ))}
        <HeaderColumnResizer />
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
  const t = useT();
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
      aria-label={t("arrangement.view.resize_headers")}
      title={t("arrangement.view.resize_headers_hint")}
      onPointerDown={onPointerDown}
      onDoubleClick={() => useArrangementUi.getState().setHeaderWidth(HEADER_WIDTH)}
    />
  );
}

/**
 * Where a dragged track header will land (a line between rows); for a return over the
 * regular tracks, the zone it can't be dropped in.
 */
function TrackDropLine() {
  const y = useArrangementUi((s) => (s.trackDrag && !s.trackDrag.into ? s.trackDrag.y : null));
  const blockedTop = useArrangementUi((s) => s.trackDrag?.blocked?.top ?? null);
  const blockedHeight = useArrangementUi((s) => s.trackDrag?.blocked?.height ?? 0);
  if (blockedTop !== null)
    return <div className="eth-arr__track-drop-blocked" style={{ top: blockedTop, height: blockedHeight }} data-testid="track-drop-blocked" />;
  return y === null ? null : <div className="eth-arr__track-drop" style={{ top: y }} data-testid="track-drop-line" />;
}

/**
 * Bar/beat lines behind the lanes, on a canvas one pixel tall stretched to the full height
 * (vertical lines look the same along y), moved and stretched by `BeatCanvas` while panning
 * and zooming. As one element per line they were restyled and repainted on every frame.
 */
function GridLayer() {
  const grid = useArrangementUi((s) => s.grid);
  const tempo = useTempoMap();
  const text = useThemeColor("text");
  const draw = (ctx: CanvasRenderingContext2D | null, win: BeatWindow, canvas: HTMLCanvasElement) => {
    const step = resolveGrid(grid, win.ppb, tempo.signatureAt(win.from));
    const lines = step ? gridLines(tempo, { start: win.from, end: win.to }, step, 8000) : [];
    canvas.dataset.count = String(lines.length);
    if (!ctx) return;
    // The former line colors: the text color at 16% (bars), 7% (beats), 3.5% (subdivisions).
    ctx.fillStyle = text;
    for (const l of lines) {
      ctx.globalAlpha = l.level === "bar" ? 0.16 : l.level === "sub" ? 0.035 : 0.07;
      ctx.fillRect(Math.round((l.beats - win.from) * win.ppb), 0, 1, 1);
    }
    ctx.globalAlpha = 1;
  };
  return (
    <BeatCanvas
      view={arrangementView}
      height={1} // eth-allow-hardcoded: one row, stretched to the content height
      stretchY
      draw={draw}
      deps={[grid, tempo, text]}
      className="eth-arr__grid"
      testId="arrangement-grid"
    />
  );
}

/**
 * The project loop region, shaded across the lanes when the loop is on. Positioned outside
 * React with a transform (no re-render, layout or repaint while panning and zooming).
 */
function LoopLayer() {
  const enabled = useProjectStore((s) => s.project?.settings.loop_enabled ?? false);
  const region = useProjectStore((s) => s.project?.settings.loop_region ?? null);
  const ref = useRef<HTMLDivElement>(null);
  const start = region?.start ?? 0;
  const end = region?.end ?? 0;
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const apply = () => {
      const v = arrangementView.getState();
      const x = (start - v.scrollBeats) * v.pxPerBeat;
      const w = Math.max(0, (end - start) * v.pxPerBeat);
      el.style.transform = `translateX(${x}px) scaleX(${w})`;
    };
    apply();
    return arrangementView.subscribe(apply);
  }, [start, end, enabled]);
  if (!enabled || !region) return null;
  return <div ref={ref} className="eth-arr__loop" data-testid="loop-region" />;
}

function NewTrackDropHint() {
  const t = useT();
  const hint = useArrangementUi((s) => (s.dropHint && s.dropHint.track === null ? s.dropHint.at : null));
  const pending = useArrangementUi((s) => s.imports.find((i) => i.track === null));
  const empty = useProjectStore((s) => !s.project || Object.keys(s.project.clips).length === 0);
  const mobile = useMobileUi((s) => s.enabled);
  // A shared project's viewer takes no drops.
  if (useReadOnly()) return null;
  if (hint === null && pending) return <ImportPlaceholder item={pending} />;
  // Touch screens: no file drops or browser there; clips come from a double-tap.
  if (hint === null && mobile) return empty ? <span className="eth-arr__drop-label">{t("arrangement.view.double_tap_hint")}</span> : null;
  if (hint === null) {
    return <span className="eth-arr__drop-label">{empty ? t("arrangement.view.empty_hint") : t("arrangement.view.drop_hint")}</span>;
  }
  return <span className="eth-arr__drop-label eth-arr__drop-label--active">{t("arrangement.view.drop_create")}</span>;
}
