import clsx from "clsx";
import { surfaceMenuEntries } from "@/ext/surfaces";
import { useStore } from "zustand";
import { memo, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import { AudioLines, ChevronDown, Circle, CornerDownRight, Folder, Headphones, Piano, Speaker, SlidersVertical, Spline, Volume2, VolumeX } from "lucide-react";
import type { Beats, Color, Track, TrackId } from "@/generated";
import { promptForInputIfNone } from "@/features/audio-settings";
import { animateGroupFold, AutomationToggleButton, TrackAutomationLanes } from "@/features/automation";
import { groupsTrackMenu, VcaLane } from "@/features/groups";
import { FreezeHeaderStatus, inertTitle, useFrozenAncestor, withFreezeTrackEntries } from "@/features/freeze";
import { LiveRecordLane } from "@/features/recording/live/LiveRecordLane";
import { tapArmBlocked } from "@/features/recording/tapRecording";
import { TakeLanes, TakesToggle, trackTakeEntries } from "@/features/comping";
import { templateTrackEntries } from "@/features/templates";
import { meterPosition, openContextMenu, ReadOnlySurface, setDragCursor, type ContextMenuEntry } from "@/kit";
import { shortcutLabel } from "@/features/keymap";
import { isReadOnly, listenBlockedReason, useProjectStore, useTrackMeter, useViewer, defaultTrackName } from "@/state";
import { pxToBeats, resolveGrid, selectModeFromEvent, snapToGrid, itemSelection, useTempoMap, useTimelineView } from "@/timeline";
import { cmd } from "@/transport";
import { trackSoloCommand } from "@/domain/soloCommands";
import { additiveClick } from "@/domain/soloPlan";
import { selectTrackEntity, trackMenu } from "./actions";
import { hasClipboard, pasteClips } from "./clipboard";
import { onLaneInsertPointerDown, type InsertSpan } from "./clipInsert";
import { colorCss, inkOn } from "./helpers";
import { sendEdit, useArrangement } from "./context";
import { beatsCss, settledZoom, subscribeSettledZoom, useSettledZoom, widthCss } from "./laneGeometry";
import { midiTarget } from "@/features/midi-learn/targets";
import { cancelDraw, scheduleDraw } from "./drawQueue";
import { HeaderPan } from "./HeaderPan";
import { InstrumentPick } from "./InstrumentPick";
import { HeaderVolume } from "./HeaderVolume";
import { sameTrackExceptVolume } from "./trackEquality";
import { useTrackDisplayName } from "./trackDisplayName";
import { onTrackHeaderPointerDown } from "./trackDrag";
import { mainHeight, TRACK_HEIGHT_STEP, type Row } from "./layout";
import { arrangementView, useArrangementUi, type PendingImport } from "./uiStore";
import { formatNumber, Trans, useT } from "@/i18n";

const INDENT_PX = 12;

// Rows stack in flow and nothing below reads `y`: rows that only move (automation lanes
// animating above them) don't re-render.
export const TrackRow = memo(
  function TrackRow({ row }: { row: Row }) {
    return <RealTrackRow row={row} />;
  },
  (a, b) => sameRowExceptY(a.row, b.row),
);

function sameRowExceptY(a: Row, b: Row): boolean {
  return (
    sameTrackExceptVolume(a.track, b.track) && a.depth === b.depth && a.laneHeight === b.laneHeight && a.height === b.height && a.takesHeight === b.takesHeight
  );
}

function RealTrackRow({ row }: { row: Row }) {
  const inert = useFrozenAncestor(row.track.id);
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const grid = useArrangementUi((s) => s.grid);
  // A selected track's header glows past its row (see .eth-arr-row--selected).
  const selected = useArrangementUi((s) => s.selectedTracks.has(row.track.id));
  // The arranger canvas covers the scrolling rows, not the pinned master row: the master's
  // automation lanes draw themselves.
  const gl = row.track.kind !== "Master";
  // The lane part doesn't depend on the automation height (animated per frame).
  const { track, depth, laneHeight, takesHeight = 0 } = row;
  const mainRow = useMemo(() => row, [track, depth, laneHeight, takesHeight]); // eslint-disable-line react-hooks/exhaustive-deps
  const main = useMemo(
    () => (
      <>
        <div className="eth-arr-row__main" style={{ height: mainHeight(mainRow) }}>
          <TrackHeader row={mainRow} />
          {/* On the edge between the header column and the lanes (outside the card). */}
          <HeaderMeter track={track.id} height={mainHeight(mainRow)} />
          <ResizeHandle row={mainRow} />
          {track.kind === "Group" ? <GroupLane track={track} /> : track.kind === "Vca" ? <VcaLane track={track} /> : <TrackLane track={track} />}
        </div>
        {/* v0.2 (`comping`): expanded take lanes, part of the row's lane height. */}
        {takesHeight > 0 && <TakesSection track={track} />}
      </>
    ),
    [mainRow, track, takesHeight],
  );
  return (
    <div
      className={clsx("eth-arr-row", row.track.freeze && "eth-arr-row--frozen", inert && "eth-arr-row--inert", selected && "eth-arr-row--selected")}
      style={{ height: row.height }}
      data-track={row.track.id}
    >
      {main}
      {/* Automation slot: its height is fed to `layoutRows` via `useAutomationHeight`. */}
      <div className="eth-arr-row__automation" data-slot="automation" data-track={row.track.id}>
        <ReadOnlySurface>
          <TrackAutomationLanes trackId={row.track.id} view={arrangementView} headerWidth={headerWidth} grid={grid} gl={gl} />
        </ReadOnlySurface>
      </div>
    </div>
  );
}

/**
 * Shows / hides the track's automation lanes (under its row). A dot marks a track that has
 * automation even while its lanes are hidden.
 */
function AutomationToggle({ track }: { track: Track }) {
  return (
    <AutomationToggleButton trackId={track.id} trackName={track.name} className="eth-arr-header__toggle eth-arr-header__automation">
      <Spline />
    </AutomationToggleButton>
  );
}

const TRACK_ICONS: Record<Track["kind"], ReactNode> = {
  Midi: <Piano />,
  Audio: <AudioLines />,
  Group: <Folder />,
  Return: <CornerDownRight />,
  Master: <Speaker />,
  // v0.2: VCA faders.
  Vca: <SlidersVertical />,
};

/**
 * A round badge in the track color with the track type's icon, in dark or light ink,
 * whichever contrasts more with the color (`inkOn`).
 */
function TrackBadge({ kind, color, compact }: { kind: Track["kind"]; color: Color; compact: boolean }) {
  const t = useT();
  return (
    <span
      className="eth-arr-header__badge"
      aria-hidden
      title={t("arrangement.header.kind", { kind: kind.toLowerCase() })}
      // Compact (a short lane): no circle, the icon in the track color (CSS).
      style={compact ? undefined : { color: inkOn(color) }}
    >
      {TRACK_ICONS[kind]}
    </span>
  );
}

/** Below this lane height (px) the header's badge is the icon alone, without its circle. */
const COMPACT_HEADER_HEIGHT = 38;

function TrackHeader({ row }: { row: Row }) {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const t = useT();
  const { track, depth } = row;
  const ctx = useArrangement();
  const { transport } = ctx;
  const dragging = useArrangementUi((s) => s.trackDrag?.track === track.id);
  const dropInto = useArrangementUi((s) => s.trackDrag?.into === track.id);
  const dropBlocked = useArrangementUi((s) => s.trackDrag?.track === track.id && !!s.trackDrag.blocked);
  // Highlighted only as the arrangement's selected entity (not while one of its clips is).
  const selected = useArrangementUi((s) => s.selectedTracks.has(track.id));
  // A selected clip on this track gives its header the hover look (not the selected one).
  const clipSelected = useStore(itemSelection, (s) => {
    const clips = useProjectStore.getState().project?.clips;
    if (!clips) return false;
    for (const id of s.selected.clip) if (clips[id]?.track === track.id) return true;
    return false;
  });
  const armed = useProjectStore((s) => s.armedTracks.includes(track.id));
  const inert = useFrozenAncestor(track.id);
  const folded = useArrangementUi((s) => s.folded.has(track.id));
  const [renaming, setRenaming] = useState(false);
  // An unnamed MIDI track shows its instrument's name.
  const displayName = useTrackDisplayName(track);
  const { mute, solo } = track.mixer;
  const canArm = track.kind === "Audio" || track.kind === "Midi";
  const armBlocked = tapArmBlocked(transport.kind, track.input, armed);
  const stop = (e: MouseEvent) => e.stopPropagation();
  // Too short for the round badge: the icon alone, in the track color.
  const compact = mainHeight(row) < COMPACT_HEADER_HEIGHT;
  // A shared project's viewer: Solo / Mute play the share's stems (tracks with a stem, groups
  // holding some); nothing else in the header edits.
  const viewer = useViewer();
  const readOnly = viewer !== null;
  const listenBlocked = listenBlockedReason(viewer, track.id);

  return (
    <>
      {/* The selected card's glow: outside the header (whose clip-path cuts its contents at the
        card's edges), behind it. */}
      {selected && (
        <div
          className="eth-arr-header__glow"
          aria-hidden
          style={{
            width: headerWidth,
            height: mainHeight(row),
            ["--eth-track-color" as string]: colorCss(track.color),
            ["--eth-track-depth" as string]: depth,
          }}
        />
      )}
      {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-noninteractive-element-interactions -- a pointer shortcut: the keyboard selects tracks with the command palette (go to track) */}
      <div
        className={clsx(
          "eth-arr-header",
          selected && "eth-arr-header--selected",
          dragging && "eth-arr-header--dragging",
          dropInto && "eth-arr-header--drop-into",
          dropBlocked && "eth-arr-header--drop-blocked",
          compact && "eth-arr-header--compact",
          inert && "eth-arr-header--inert",
          `eth-arr-header--${track.kind.toLowerCase()}`,
        )}
        data-clip-selected={clipSelected || undefined}
        data-track={track.id}
        data-tour="track-header"
        style={{
          width: headerWidth,
          paddingLeft: 14 + depth * INDENT_PX,
          ["--eth-track-color" as string]: colorCss(track.color),
          ["--eth-track-depth" as string]: depth,
        }}
        onPointerDown={(e) => {
          stop(e);
          if (!renaming && !readOnly) onTrackHeaderPointerDown(e, track, ctx);
        }}
        onClick={(e) => selectTrackEntity(track.id, selectModeFromEvent(e))}
        onContextMenu={(e) => {
          const items = trackMenu(transport, track);
          const selected = [...useArrangementUi.getState().selectedTracks];
          // Group/ungroup and VCA entries (after the owner's track menu + freeze entries).
          openContextMenu(e, [
            // comping: take entries go before the track menu's last group.
            // Take entries are per track: only for a single-track menu.
            ...withFreezeTrackEntries(selected.length > 1 ? items : withTakeEntries(items, trackTakeEntries(transport, track.id)), transport, track, selected),
            ...groupsTrackMenu(transport, track, useArrangementUi.getState().selectedTracks),
            // templates: save as / insert a track template.
            ...templateTrackEntries(useProjectStore.getState().project, track, useArrangementUi.getState().selectedTracks),
          ]);
        }}
        role="group"
        aria-label={t("arrangement.header.track", { name: track.name })}
      >
        {track.kind === "Group" ? (
          // A group's badge is its fold button: the chevron on the track-color circle.
          <button
            type="button"
            className="eth-arr-header__badge eth-arr-header__fold"
            data-folded={folded || undefined}
            aria-expanded={!folded}
            aria-label={folded ? t("arrangement.header.unfold_named", { name: track.name }) : t("arrangement.header.fold_named", { name: track.name })}
            title={folded ? t("arrangement.header.unfold") : t("arrangement.header.fold")}
            style={compact ? undefined : { color: inkOn(track.color) }}
            onClick={(e) => {
              stop(e);
              animateGroupFold(track.id, folded);
              useArrangementUi.getState().toggleFold(track.id);
            }}
          >
            <ChevronDown />
          </button>
        ) : (
          <TrackBadge kind={track.kind} color={track.color} compact={compact} />
        )}
        <span className="eth-arr-header__main">
          {renaming ? (
            <TrackNameInput
              name={displayName}
              onDone={(name) => {
                setRenaming(false);
                if (name === null) return;
                // Empty: back to its default name ("3 midi": shown as its instrument's name again).
                const project = useProjectStore.getState().project;
                const next = name.trim() || (project ? defaultTrackName(project, track) : "");
                if (next && next !== track.name) {
                  void sendEdit(
                    transport,
                    cmd("Track", {
                      type: "Rename",
                      id: track.id,
                      name: next,
                    }),
                  );
                }
              }}
            />
          ) : (
            <span
              className="eth-arr-header__name eth-user-text"
              title={t("arrangement.header.name_title", { name: displayName })}
              onDoubleClick={(e) => {
                stop(e);
                if (!readOnly) setRenaming(true);
              }}
            >
              {displayName}
            </span>
          )}
        </span>
        {/* The volume and pan at the header's right end, the buttons (always shown, small) under
          them. The buttons come first in the DOM (the touch layouts lay them out before the
          volume and pan: mobile.css); arrangement.css puts them below. */}
        <span className="eth-arr-header__controls">
          {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- only keeps its events from reaching the track header around it */}
          <span className="eth-arr-header__buttons" onClick={stop}>
            <FreezeHeaderStatus track={track} transport={transport} />
            {/* First: the takes button must not move the others (their positions are the same
            with or without take lanes). */}
            {canArm && <TakesToggle track={track} className="eth-arr-header__toggle" />}
            <button
              type="button"
              className="eth-arr-header__toggle eth-arr-header__mute"
              {...midiTarget({ type: "TrackMute", track: track.id })}
              aria-pressed={mute}
              aria-label={t("arrangement.header.mute_named", { name: track.name })}
              disabled={!!inert || !!listenBlocked}
              title={inert ? inertTitle(inert) : (listenBlocked ?? (mute ? t("arrangement.header.unmute") : t("arrangement.header.mute")))}
              onClick={() => void sendEdit(transport, cmd("Mixer", { type: "SetMute", track: track.id, mute: !mute }))}
            >
              {mute ? <VolumeX /> : <Volume2 />}
            </button>
            {track.kind !== "Master" && (
              <button
                type="button"
                className="eth-arr-header__toggle eth-arr-header__solo"
                {...midiTarget({ type: "TrackSolo", track: track.id })}
                aria-pressed={solo}
                aria-label={t("arrangement.header.solo_named", { name: track.name })}
                disabled={!!inert || !!listenBlocked}
                title={inert ? inertTitle(inert) : (listenBlocked ?? t("arrangement.header.solo_title"))}
                onClick={(e) =>
                  void sendEdit(transport, trackSoloCommand(Object.values(useProjectStore.getState().project?.tracks ?? {}), track.id, additiveClick(e)))
                }
              >
                <Headphones />
              </button>
            )}
            {canArm && !readOnly && (
              <button
                type="button"
                className="eth-arr-header__toggle eth-arr-header__arm"
                {...midiTarget({ type: "TrackArm", track: track.id })}
                aria-pressed={armed}
                aria-label={t("arrangement.header.arm_named", { name: track.name })}
                // Browser build: a tapped track can't record (no capture); say why.
                aria-disabled={armBlocked || inert ? true : undefined}
                style={armBlocked || inert ? { opacity: "var(--eth-opacity-disabled)", cursor: "not-allowed" } : undefined}
                title={(inert ? inertTitle(inert) : armBlocked) ?? t("arrangement.header.arm_title")}
                onClick={(e) =>
                  !armBlocked &&
                  !inert &&
                  void transport
                    .send(
                      cmd("Recording", {
                        type: "Arm",
                        track: track.id,
                        armed: !armed,
                        exclusive: !(e.ctrlKey || e.metaKey),
                      }),
                    )
                    // Arming an audio track with no input device open: offer to choose one.
                    .then(() => {
                      if (!armed && track.kind === "Audio" && track.input.type !== "Track") void promptForInputIfNone(transport);
                    })
                    .catch((err: unknown) => console.warn("arm failed", err))
                }
              >
                <Circle />
              </button>
            )}
            <AutomationToggle track={track} />
          </span>
          <span className="eth-arr-header__mix">
            <HeaderVolume track={track} />
            <HeaderPan track={track} />
          </span>
        </span>
        {track.kind === "Midi" && !readOnly && <InstrumentPick track={track} />}
      </div>
    </>
  );
}

/** Inline rename field: Enter or blur commits, Escape cancels (`onDone(null)`). */
function TrackNameInput({ name, onDone }: { name: string; onDone: (name: string | null) => void }) {
  const [value, setValue] = useState(name);
  const t = useT();
  const done = useRef(false);
  const finish = (v: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(v);
  };
  return (
    <input
      className="eth-arr-header__rename"
      aria-label={t("arrangement.header.track_name")}
      value={value}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onChange={(e) => setValue(e.target.value)}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(value);
        else if (e.key === "Escape") finish(null);
      }}
      onBlur={() => finish(value)}
    />
  );
}

/** Live level along the header's right edge (its own component: re-renders at meter rate). */
function HeaderMeter({ track, height }: { track: TrackId; height: number }) {
  const meter = useTrackMeter(track);
  const level = meter ? meterPosition(Math.max(meter.peak[0], meter.peak[1])) : 0;
  return (
    <span className="eth-arr-header__meter" style={{ height }} aria-hidden>
      <span className="eth-arr-header__meter-fill" style={{ transform: `scaleY(${level})` }} />
    </span>
  );
}

/**
 * Bottom edge of a track's header (not the lane, so clips keep their edges): drag to resize
 * the track in `TRACK_HEIGHT_STEP` increments (alt: free), double-click to reset it.
 */
function ResizeHandle({ row }: { row: Row }) {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const t = useT();
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    const startY = e.clientY;
    const startH = mainHeight(row);
    const ui = useArrangementUi.getState();
    const move = (ev: globalThis.PointerEvent) => {
      const h = startH + ev.clientY - startY;
      ui.setHeight(row.track.id, ev.altKey ? h : Math.round(h / TRACK_HEIGHT_STEP) * TRACK_HEIGHT_STEP);
    };
    const done = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", done);
      window.removeEventListener("pointercancel", done);
      setDragCursor(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", done);
    window.addEventListener("pointercancel", done);
    setDragCursor("ns-resize");
  };
  return (
    <div
      className="eth-arr-row__resize"
      style={{ top: mainHeight(row) - 3, width: headerWidth }}
      onPointerDown={onPointerDown}
      onDoubleClick={(e) => {
        e.stopPropagation();
        useArrangementUi.getState().setHeight(row.track.id, null);
      }}
      role="separator"
      aria-orientation="horizontal"
      aria-label={t("arrangement.header.resize_named", { name: row.track.name })}
      title={t("arrangement.header.resize_title")}
    />
  );
}

/**
 * What the lanes render against, without re-rendering on every scroll frame: the zoom, and
 * a coarse visible window that only moves after a whole viewport of scrolling. Lane
 * content is laid out from `origin` (`vp.scrollBeats`) and slid by `LaneLayer`, which
 * follows the exact scroll position through the DOM. Culling and clip canvases use
 * `visible`, which always covers the view with a viewport of margin on each side.
 */
function useLaneView() {
  // Zoom: the settled one (see laneGeometry.ts); LaneLayer stretches lanes to the live zoom.
  const pxPerBeat = useSettledZoom(arrangementView);
  const width = useTimelineView(arrangementView, (s) => s.widthPx) || 4000;
  const span = width / pxPerBeat;
  const step = useTimelineView(arrangementView, (s) => Math.floor(s.scrollBeats / span));
  return useMemo(() => {
    const origin = Math.max(0, (step - 1) * span);
    return {
      vp: { pxPerBeat, scrollBeats: origin },
      // Two spans past the view at least: until lanes re-render at the new zoom, zooming out
      // shows up to RENDER_DRIFT (2) spans.
      visible: { start: origin, end: (step + 3) * span },
    };
  }, [pxPerBeat, step, span]);
}

/**
 * Positions lane content (laid out in beats from `origin`, see laneGeometry.ts) at the live
 * zoom and scroll, outside React: `--ppb` is the settled zoom (written when it changes), the
 * transform carries the scroll and the live/`--ppb` ratio (written on every view change).
 * A new settled zoom restyles every clip of the lane: lanes take turns over a few frames
 * (drawQueue.ts) instead of all restyling in one long frame, each staying stretched to the
 * live zoom until then.
 */
function LaneLayer({ origin, children }: { origin: Beats; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const view = arrangementView;
    let ppb = NaN;
    const position = () => {
      const s = view.getState();
      const scale = s.pxPerBeat / ppb;
      el.style.transform =
        scale === 1 ? `translateX(${(origin - s.scrollBeats) * s.pxPerBeat}px)` : `translateX(${(origin - s.scrollBeats) * s.pxPerBeat}px) scaleX(${scale})`;
    };
    const restyle = () => {
      const settled = settledZoom(view);
      if (settled === ppb) return;
      ppb = settled;
      el.style.setProperty("--ppb", `${settled}px`);
      position();
    };
    restyle();
    const offView = view.subscribe(position);
    const offSettled = subscribeSettledZoom(view, () => scheduleDraw(el, restyle, el.childElementCount));
    return () => {
      offView();
      offSettled();
      cancelDraw(el);
    };
  }, [origin]);
  return (
    <div className="eth-arr-lane__layer" ref={ref}>
      {children}
    </div>
  );
}

/**
 * The clip lane of an audio or MIDI (or return/master: empty) track. The arranger canvas
 * (gl/) draws and hit-tests the clips; the lane keeps the DOM overlays (MIDI insert preview,
 * drop hint, imports, live recording) and the presses that reach it (insert, paste menu).
 */
function TrackLane({ track }: { track: Track }) {
  const ctx = useArrangement();
  const dropHint = useArrangementUi((s) => (s.dropHint?.track === track.id ? s.dropHint.at : null));
  const imports = useArrangementUi((s) => s.imports);
  const tempo = useTempoMap();
  const t = useT();
  const { vp, visible } = useLaneView();

  const [insert, setInsert] = useState<InsertSpan | null>(null);

  return (
    <div
      className={clsx("eth-arr-lane", track.kind !== "Audio" && track.kind !== "Midi" && "eth-arr-lane--no-clips")}
      data-lane={track.id}
      onPointerDown={(e) => {
        if (track.kind === "Midi" && !isReadOnly()) onLaneInsertPointerDown(e, track.id, ctx, setInsert);
      }}
      onContextMenu={(e) => {
        // Empty space (or a clip's body, which lets clicks through): paste here.
        const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
        const s = arrangementView.getState();
        const step = resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
        const at = Math.max(0, snapToGrid(pxToBeats(x, s), e.altKey ? null : step, tempo, "floor"));
        openContextMenu(e, [
          {
            label: t("arrangement.lane.paste"),
            shortcut: shortcutLabel("edit.paste"),
            disabled: !hasClipboard(),
            onSelect: () => void pasteClips(ctx.transport, at, track.id),
          },
          ...surfaceMenuEntries({ kind: "arranger" }, e),
        ]);
      }}
    >
      <LaneLayer origin={vp.scrollBeats}>
        {insert && (
          <div
            className="eth-clip eth-clip--ghost"
            data-testid="insert-preview"
            style={{
              left: beatsCss(insert.start - vp.scrollBeats),
              width: widthCss(insert.length),
              ["--eth-clip-color" as string]: colorCss(track.color),
            }}
          />
        )}
        {dropHint !== null && <div className="eth-arr-lane__drop-hint" style={{ left: beatsCss(dropHint - vp.scrollBeats) }} />}
        {imports.map((i) => (i.track === track.id ? <ImportPlaceholder key={i.id} item={i} style={{ left: beatsCss(i.at - vp.scrollBeats) }} /> : null))}
        {(track.kind === "Audio" || track.kind === "Midi") && (
          <LiveRecordLane transport={ctx.transport} track={track.id} color={track.color} origin={vp.scrollBeats} visible={visible} pxPerBeat={vp.pxPerBeat} />
        )}
      </LaneLayer>
    </div>
  );
}

/** v0.2 (`comping`): a track's take lanes (headers; the canvas draws their clips). */
function TakesSection({ track }: { track: Track }) {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  return <TakeLanes track={track} headerWidth={headerWidth} />;
}

/** Take entries (`comping`) go before the track menu's last group (Delete). */
function withTakeEntries(menu: ContextMenuEntry[], takes: ContextMenuEntry[]): ContextMenuEntry[] {
  if (takes.length === 0) return menu;
  const at = menu.lastIndexOf("separator");
  return at < 0 ? [...menu, "separator", ...takes] : [...menu.slice(0, at), "separator", ...takes, ...menu.slice(at)];
}

/** A browser drop still importing (or failed), shown where the clip will go. */
export function ImportPlaceholder({ item, style }: { item: PendingImport; style?: CSSProperties }) {
  const name = <span className="eth-user-text">{item.name}</span>;
  const text = item.error ? (
    <Trans k="arrangement.import.failed" values={{ name }} />
  ) : item.progress !== null ? (
    <Trans k="arrangement.import.progress_percent" values={{ name, percent: formatNumber(item.progress, { style: "percent", maximumFractionDigits: 0 }) }} />
  ) : (
    <Trans k="arrangement.import.progress" values={{ name }} />
  );
  return (
    <div
      className={clsx("eth-arr-import", item.error && "eth-arr-import--error")}
      style={style}
      role="status"
      title={item.error ?? undefined}
      data-testid="import-placeholder"
    >
      {text}
    </div>
  );
}

/** Group lane: the summary of its tracks' clips is drawn by the canvas (gl/layers/groupSummary.ts). */
function GroupLane({ track }: { track: Track }) {
  return <div className="eth-arr-lane eth-arr-lane--group" data-lane={track.id} />;
}

/**
 * Clips dragged below the last track: the new tracks the drop will create, as
 * ghost rows (in the draft row's style) right under the last track, with the dragged clips
 * where they will land. Not interactive (the drag follows the pointer itself).
 */
export function NewTrackGhostRows() {
  const preview = useArrangementUi((s) => (s.preview?.newTracks?.length ? s.preview : null));
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const height = useArrangementUi((s) => s.defaultHeight);
  const t = useT();
  // The arranger canvas draws the ghost lanes' clips (gl/layers/ghostLanes.ts).
  if (!preview) return null;
  return (
    <div className="eth-arr__ghost-rows" data-testid="new-track-ghosts">
      {preview.newTracks!.map((lane) => (
        <div key={lane.id} className="eth-arr-row eth-arr-row--draft eth-arr-row--ghost" style={{ height }} data-testid="new-track-ghost" data-kind={lane.kind}>
          <div className="eth-arr-row__main" style={{ height }}>
            <div className="eth-arr-header eth-arr-header--draft" style={{ width: headerWidth }}>
              <span className="eth-arr-header__name">{lane.kind === "Midi" ? t("arrangement.ghost.new_midi") : t("arrangement.ghost.new_audio")}</span>
            </div>
            <div className="eth-arr-lane eth-arr-lane--ghost" />
          </div>
        </div>
      ))}
    </div>
  );
}
