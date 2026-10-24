import clsx from "clsx";
import { surfaceMenuEntries } from "@/ext/surfaces";
import { useContributions } from "@/ext";
import { useStore } from "zustand";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import { AudioLines, ChevronDown, Circle, CornerDownRight, Folder, Headphones, Piano, Speaker, SlidersVertical, Spline, Volume2, VolumeX } from "lucide-react";
import type { Beats, Track, TrackId } from "@/generated";
import { promptForInputIfNone } from "@/features/audio-settings";
import { TrackRecordSetup } from "@/features/recording";
import { TrackPluginLoading, useTrackLoading } from "@/features/plugins";
import { animateGroupFold, AutomationToggleButton, TrackAutomationLanes } from "@/features/automation";
import { groupsTrackMenu, VcaLane } from "@/features/groups";
import { FreezeHeaderStatus, inertTitle, useFrozenAncestor, withFreezeTrackEntries } from "@/features/freeze";
import { LiveRecordLane } from "@/features/recording/live/LiveRecordLane";
import { tapArmBlocked } from "@/features/recording/tapRecording";
import { TakeLanes, TakesToggle, trackTakeEntries } from "@/features/comping";
import { templateTrackEntries } from "@/features/templates";
import { meterPosition, openContextMenu, Popover, ReadOnlySurface, setDragCursor, type ContextMenuEntry } from "@/kit";
import { ColorSwatches } from "@/features/colors";
import { shortcutLabel } from "@/features/keymap";
import { useShellStore } from "@/state/shell";
import { isReadOnly, listenBlockedReason, useProjectStore, useTrackMeter, useViewer, defaultTrackName } from "@/state";
import { pxToBeats, resolveGrid, selectModeFromEvent, snapToGrid, itemSelection, useTempoMap, useTimelineView } from "@/timeline";
import { cmd } from "@/transport";
import { trackSoloCommand } from "@/domain/soloCommands";
import { additiveClick, muteOverriddenBySolo } from "@/domain/soloPlan";
import { selectTrackEntity, trackMenu } from "./actions";
import { hasClipboard, pasteClips } from "./clipboard";
import { onLaneInsertPointerDown, type InsertSpan } from "./clipInsert";
import { colorCss } from "./helpers";
import { isCompactLane } from "./laneBodyGeometry";
import { sendEdit, useArrangement } from "./context";
import { beatsCss, settledZoom, subscribeSettledZoom, useSettledZoom, widthCss } from "./laneGeometry";
import { midiTarget } from "@/features/midi-learn/targets";
import { cancelDraw, scheduleDraw } from "./drawQueue";
import { HeaderPan } from "./HeaderPan";
import { InstrumentPick } from "./InstrumentPick";
import { HeaderVolume } from "./HeaderVolume";
import { sameTrackExceptVolume } from "./trackEquality";
import { useTrackDisplayName } from "./trackDisplayName";
import { useGroupRailColors } from "./groupRails";
import { onTrackHeaderPointerDown } from "./trackDrag";
import { HEADER_INDENT_PX, mainHeight, NARROW_HEADER_WIDTH, TRACK_HEIGHT_STEP, type Row } from "./layout";
import { arrangementView, useArrangementUi, type PendingImport } from "./uiStore";
import { formatNumber, Trans, useT } from "@/i18n";

// Rows stack in flow and nothing below reads `y`: rows that only move (automation lanes
// animating above them) don't re-render.
/** `VcaLane`, re-rendered only when its track changes (not on a lane height change). */
const VcaLaneMemo = memo(VcaLane);

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
  // Still loading as the project opens (#260): the row looks unavailable until it is ready.
  const loading = useTrackLoading(row.track.id);
  // The arranger canvas covers the scrolling rows, not the pinned master row: the master's
  // automation lanes draw themselves.
  const gl = row.track.kind !== "Master";
  // The lane part doesn't depend on the automation height (animated per frame).
  const { track, depth, laneHeight, takesHeight = 0 } = row;
  // Inside groups: a rail per enclosing group along the header column, the innermost one's
  // also along the lane's left edge (arrangement.css), so membership reads at a glance.
  const groupColors = useGroupRailColors(track);
  const mainRow = useMemo(() => row, [track, depth, laneHeight, takesHeight]); // eslint-disable-line react-hooks/exhaustive-deps
  const main = useMemo(
    () => (
      <>
        <div className="eth-arr-row__main" style={{ height: mainHeight(mainRow) }}>
          {/* Memoized on what they show (not the height: CSS sizes them), so a vertical zoom
              step re-renders none of them unless the lane turns compact or back. */}
          <TrackHeader track={track} depth={depth} compact={isCompactLane(mainHeight(mainRow))} />
          {/* On the edge between the header column and the lanes (outside the card). */}
          <HeaderMeter track={track.id} />
          <ResizeHandle row={mainRow} />
          {track.kind === "Group" ? <GroupLane track={track} /> : track.kind === "Vca" ? <VcaLaneMemo track={track} /> : <TrackLane track={track} />}
        </div>
        {/* v0.2 (`comping`): expanded take lanes, part of the row's lane height. */}
        {takesHeight > 0 && <TakesSection track={track} />}
      </>
    ),
    [mainRow, track, depth, takesHeight],
  );
  return (
    <div
      className={clsx(
        "eth-arr-row",
        row.track.freeze && "eth-arr-row--frozen",
        inert && "eth-arr-row--inert",
        selected && "eth-arr-row--selected",
        loading && "eth-arr-row--loading",
      )}
      // The automation lanes' headers indent and tint like the track's (arrangement.css).
      style={{ height: row.height, ["--eth-track-depth" as string]: depth, ["--eth-track-color" as string]: colorCss(track.color) }}
      data-track={row.track.id}
    >
      {groupColors.length > 0 && (
        <div className="eth-arr-row__groups" aria-hidden data-testid="group-rails">
          {groupColors.map((color, level) => (
            <span key={level} className="eth-arr-row__group-rail" style={{ ["--eth-group-level" as string]: level, ["--eth-group-color" as string]: color }} />
          ))}
          <span className="eth-arr-row__group-edge" style={{ ["--eth-group-color" as string]: groupColors[groupColors.length - 1] }} />
        </div>
      )}
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
 * The empty slot of a header button a track doesn't have (no solo on the master, no record arm on
 * a group, return or vca): the same size as the button, so mute, solo and the others stay in the
 * same column on every row (#239).
 */
function HeaderSlot() {
  return <span className="eth-arr-header__toggle eth-arr-header__slot" aria-hidden />;
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
 * The track type's icon in the track color (CSS; compact, a short lane: smaller). With
 * `onPick` (an editable track's color), a double-click opens the palette under it.
 */
function TrackBadge({ track, onPick }: { track: Track; onPick?: (color: number) => void }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const kind = t("arrangement.header.kind", { kind: track.kind.toLowerCase() });
  if (!onPick) {
    return (
      <span className="eth-arr-header__badge" aria-hidden title={kind}>
        {TRACK_ICONS[track.kind]}
      </span>
    );
  }
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      placement="bottom-start"
      aria-label={t("arrangement.header.color_named", { name: track.name })}
      trigger={() => (
        // A pointer shortcut (the inspector's color square is the keyboard's way).
        <span
          className="eth-arr-header__badge"
          aria-hidden
          title={`${kind} · ${t("arrangement.header.color_hint")}`}
          onDoubleClick={(e) => {
            e.stopPropagation();
            setOpen(true);
          }}
        >
          {TRACK_ICONS[track.kind]}
        </span>
      )}
    >
      {(close) => (
        // Picks stay in the palette (not a press / click on the track header behind it).
        // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
        <div className="eth-color-palette" onPointerDown={(e) => e.stopPropagation()} onClick={(e) => e.stopPropagation()}>
          <ColorSwatches
            label={t("arrangement.header.color_named", { name: track.name })}
            value={track.color}
            onPick={(c) => {
              if (c !== null) onPick(c);
              close();
            }}
          />
        </div>
      )}
    </Popover>
  );
}

/** Whether `group` contains `track` (as a child, or deeper). */
function hasAncestor(track: Track, group: TrackId): boolean {
  const tracks = useProjectStore.getState().project?.tracks;
  let parent = track.parent;
  for (let depth = 0; parent && tracks && depth < 64; depth++) {
    if (parent === group) return true;
    parent = tracks[parent]?.parent ?? null;
  }
  return false;
}

const TrackHeader = memo(function TrackHeader({ track, depth, compact }: { track: Track; depth: number; compact: boolean }) {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const t = useT();
  // Inside a group whose header is hovered (at any depth): the hover look too.
  const inHoveredGroup = useArrangementUi((s) => s.hoveredGroup !== null && hasAncestor(track, s.hoveredGroup));
  const ctx = useArrangement();
  const { transport } = ctx;
  const dragging = useArrangementUi((s) => s.trackDrag?.track === track.id);
  const dropInto = useArrangementUi((s) => s.trackDrag?.into === track.id);
  // Device cards dragged over the track (they land on its chain).
  const deviceDrop = useArrangementUi((s) => s.deviceDrop === track.id);
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
  // Still loading as the project opens (#260): dimmed, its mute, solo and arm inactive (with
  // why); it can still be selected, renamed and opened.
  const loading = useTrackLoading(track.id);
  const loadingTitle = loading ? t("arrangement.header.loading") : null;
  const folded = useArrangementUi((s) => s.folded.has(track.id));
  const [renaming, setRenaming] = useState(false);
  const [recordSetup, setRecordSetup] = useState(false);
  // An unnamed MIDI track shows its instrument's name.
  const displayName = useTrackDisplayName(track);
  const { mute, solo } = track.mixer;
  const canArm = track.kind === "Audio" || track.kind === "Midi";
  const armBlocked = tapArmBlocked(transport.kind, track.input, armed);
  const stop = (e: MouseEvent) => e.stopPropagation();
  // A shared project's viewer: Solo / Mute play the share's stems (tracks with a stem, groups
  // holding some); nothing else in the header edits.
  const viewer = useViewer();
  const readOnly = viewer !== null;
  const listenBlocked = listenBlockedReason(viewer, track.id);
  // Turning compact or back, the buttons change places: for that frame they show or hide at
  // once (their fade is for hover), never fading out where they were.
  const [wasCompact, setWasCompact] = useState(compact);
  const [relayout, setRelayout] = useState(false);
  if (wasCompact !== compact) {
    setWasCompact(compact);
    setRelayout(true);
  }
  useEffect(() => {
    if (!relayout) return;
    const frame = requestAnimationFrame(() => setRelayout(false));
    return () => cancelAnimationFrame(frame);
  }, [relayout]);

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
          (dropInto || deviceDrop) && "eth-arr-header--drop-into",
          dropBlocked && "eth-arr-header--drop-blocked",
          compact && "eth-arr-header--compact",
          headerWidth < NARROW_HEADER_WIDTH && "eth-arr-header--narrow",
          inert && "eth-arr-header--inert",
          loading && "eth-arr-header--loading",
          `eth-arr-header--${track.kind.toLowerCase()}`,
        )}
        data-clip-selected={clipSelected || undefined}
        data-loading={loading || undefined}
        data-group-hover={inHoveredGroup || undefined}
        data-relayout={relayout || undefined}
        onPointerEnter={track.kind === "Group" ? () => useArrangementUi.getState().setHoveredGroup(track.id) : undefined}
        onPointerLeave={
          track.kind === "Group"
            ? () => {
                if (useArrangementUi.getState().hoveredGroup === track.id) useArrangementUi.getState().setHoveredGroup(null);
              }
            : undefined
        }
        data-track={track.id}
        data-tour="track-header"
        style={{
          width: headerWidth,
          paddingLeft: 14 + depth * HEADER_INDENT_PX,
          ["--eth-track-color" as string]: colorCss(track.color),
          ["--eth-track-depth" as string]: depth,
        }}
        onPointerDown={(e) => {
          stop(e);
          if (!renaming && !readOnly) onTrackHeaderPointerDown(e, track, ctx);
        }}
        onClick={(e) => {
          selectTrackEntity(track.id, selectModeFromEvent(e));
          // A header click expands the inspector on the track (the press collapsed the drawer:
          // ArrangementView).
          useShellStore.getState().setCollapsed("right", false);
        }}
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
            onClick={(e) => {
              stop(e);
              animateGroupFold(track.id, folded);
              useArrangementUi.getState().toggleFold(track.id);
            }}
          >
            <ChevronDown />
          </button>
        ) : (
          <TrackBadge
            track={track}
            onPick={
              readOnly || track.kind === "Master" ? undefined : (color) => void sendEdit(transport, cmd("Track", { type: "SetColor", id: track.id, color }))
            }
          />
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
          {/* Loading: a disabled button shows no tooltip in every browser; the box says why. */}
          {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions -- only keeps its events from reaching the track header around it */}
          <span className="eth-arr-header__buttons" onClick={stop} title={loadingTitle ?? undefined}>
            <FreezeHeaderStatus track={track} transport={transport} />
            {/* First: the takes button must not move the others (their positions are the same
            with or without take lanes). */}
            {canArm && <TakesToggle track={track} className="eth-arr-header__toggle" />}
            <button
              type="button"
              className="eth-arr-header__toggle eth-arr-header__mute"
              {...midiTarget({ type: "TrackMute", track: track.id })}
              aria-pressed={mute}
              data-solo-override={muteOverriddenBySolo(mute, solo) || undefined}
              aria-label={t("arrangement.header.mute_named", { name: track.name })}
              disabled={!!inert || loading || !!listenBlocked}
              title={
                inert
                  ? inertTitle(inert)
                  : (loadingTitle ??
                    listenBlocked ??
                    (muteOverriddenBySolo(mute, solo) ? t("mixer.mute_overridden") : mute ? t("arrangement.header.unmute") : t("arrangement.header.mute")))
              }
              onClick={() => void sendEdit(transport, cmd("Mixer", { type: "SetMute", track: track.id, mute: !mute }))}
            >
              {mute ? <VolumeX /> : <Volume2 />}
            </button>
            {track.kind !== "Master" ? (
              <button
                type="button"
                className="eth-arr-header__toggle eth-arr-header__solo"
                {...midiTarget({ type: "TrackSolo", track: track.id })}
                aria-pressed={solo}
                aria-label={t("arrangement.header.solo_named", { name: track.name })}
                disabled={!!inert || loading || !!listenBlocked}
                title={inert ? inertTitle(inert) : (loadingTitle ?? listenBlocked ?? t("arrangement.header.solo_title"))}
                onClick={(e) =>
                  void sendEdit(transport, trackSoloCommand(Object.values(useProjectStore.getState().project?.tracks ?? {}), track.id, additiveClick(e)))
                }
              >
                <Headphones />
              </button>
            ) : (
              <HeaderSlot />
            )}
            {readOnly ? null : canArm ? (
              // Right-click: the track's input and monitoring, in a popover.
              <Popover
                open={recordSetup}
                onOpenChange={setRecordSetup}
                placement="bottom-end"
                aria-label={t("arrangement.header.record_setup_named", { name: track.name })}
                trigger={() => (
                  <button
                    type="button"
                    className="eth-arr-header__toggle eth-arr-header__arm"
                    onContextMenu={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      setRecordSetup(true);
                    }}
                    {...midiTarget({ type: "TrackArm", track: track.id })}
                    aria-pressed={armed}
                    aria-label={t("arrangement.header.arm_named", { name: track.name })}
                    // Browser build: a tapped track can't record (no capture); say why.
                    aria-disabled={armBlocked || inert || loading ? true : undefined}
                    style={armBlocked || inert || loading ? { opacity: "var(--eth-opacity-disabled)", cursor: "not-allowed" } : undefined}
                    title={(inert ? inertTitle(inert) : (loadingTitle ?? armBlocked)) ?? t("arrangement.header.arm_title")}
                    onClick={(e) =>
                      !armBlocked &&
                      !inert &&
                      !loading &&
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
              >
                {/* Picks stay in the popover (not a press / click on the track header behind it). */}
                {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions */}
                <div onPointerDown={(e) => e.stopPropagation()} onClick={(e) => e.stopPropagation()}>
                  <TrackRecordSetup track={track.id} />
                </div>
              </Popover>
            ) : (
              <HeaderSlot />
            )}
            <AutomationToggle track={track} />
          </span>
          {/* Loading: the fader and pan are inactive; the box keeps the tooltip saying why. */}
          <span className="eth-arr-header__mix" title={loadingTitle ?? undefined}>
            <span className="eth-arr-header__mix-controls" inert={loading || undefined}>
              <HeaderVolume track={track} />
              <HeaderPan track={track} />
            </span>
          </span>
        </span>
        {/* A big project opening: how many of the track's plugins are loaded. */}
        <TrackPluginLoading track={track.id} />
        {track.kind === "Midi" && !readOnly && <InstrumentPick track={track} />}
      </div>
    </>
  );
});

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

/** Live level along the header's right edge, left and right channels side by side (its own
 *  component: re-renders at meter rate). */
const HeaderMeter = memo(function HeaderMeter({ track }: { track: TrackId }) {
  const meter = useTrackMeter(track);
  const left = meter ? meterPosition(meter.peak[0]) : 0;
  const right = meter ? meterPosition(meter.peak[1]) : 0;
  return (
    <span className="eth-arr-header__meter" aria-hidden>
      <span className="eth-arr-header__meter-channel" data-channel="left">
        <span className="eth-arr-header__meter-fill" style={{ transform: `scaleY(${left})` }} />
      </span>
      <span className="eth-arr-header__meter-channel" data-channel="right">
        <span className="eth-arr-header__meter-fill" style={{ transform: `scaleY(${right})` }} />
      </span>
    </span>
  );
});

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
const TrackLane = memo(function TrackLane({ track }: { track: Track }) {
  const ctx = useArrangement();
  const dropHint = useArrangementUi((s) => (s.dropHint?.track === track.id ? s.dropHint.at : null));
  const relinkDrop = useArrangementUi((s) => (s.relinkDrop?.track === track.id ? s.relinkDrop : null));
  const imports = useArrangementUi((s) => s.imports);
  const tempo = useTempoMap();
  const t = useT();
  const { vp, visible } = useLaneView();

  const [insert, setInsert] = useState<InsertSpan | null>(null);
  const laneExts = useContributions("arrangement.lane");

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
        {relinkDrop && (
          <div
            className="eth-arr-lane__relink-hint"
            data-testid="relink-drop-hint"
            style={{ left: beatsCss(relinkDrop.start - vp.scrollBeats), width: widthCss(relinkDrop.length) }}
          >
            <span className="eth-arr-lane__relink-label">{t("arrangement.lane.drop_relink")}</span>
          </div>
        )}
        {imports.map((i) => (i.track === track.id ? <ImportPlaceholder key={i.id} item={i} style={{ left: beatsCss(i.at - vp.scrollBeats) }} /> : null))}
        {(track.kind === "Audio" || track.kind === "Midi") && (
          <>
            <LiveRecordLane transport={ctx.transport} track={track.id} color={track.color} origin={vp.scrollBeats} visible={visible} pxPerBeat={vp.pxPerBeat} />
            {/* Extensions' lane content (`@/ext` "arrangement.lane"). */}
            {laneExts.map((Lane, i) => (
              <Lane key={i} transport={ctx.transport} track={track.id} color={track.color} origin={vp.scrollBeats} visible={visible} pxPerBeat={vp.pxPerBeat} />
            ))}
          </>
        )}
      </LaneLayer>
    </div>
  );
});

/** v0.2 (`comping`): a track's take lanes (headers; the canvas draws their clips). */
const TakesSection = memo(function TakesSection({ track }: { track: Track }) {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  return <TakeLanes track={track} headerWidth={headerWidth} />;
});

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
const GroupLane = memo(function GroupLane({ track }: { track: Track }) {
  return <div className="eth-arr-lane eth-arr-lane--group" data-lane={track.id} />;
});

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
