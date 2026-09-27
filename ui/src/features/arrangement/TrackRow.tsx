import clsx from "clsx";
import { memo, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import {
  AudioLines,
  ChevronDown,
  ChevronRight,
  Circle,
  CornerDownRight,
  Folder,
  Headphones,
  Piano,
  Speaker,
  Spline,
  Volume2,
  VolumeX,
} from "lucide-react";
import type { Beats, Clip, ClipId, Color, Track, TrackId } from "@/generated";
import { promptForInputIfNone } from "@/features/audio-settings";
import { AutomationToggleButton, TrackAutomationLanes } from "@/features/automation";
import { leaveNoteEntries, withSeparator } from "@/features/collab/social";
import { LiveRecordLane } from "@/features/recording/live/LiveRecordLane";
import { MOD_KEY, meterPosition, openContextMenu, setDragCursor } from "@/kit";
import { useEditorStore, useProjectStore, useTrackMeter } from "@/state";
import { pxToBeats, resolveGrid, selectModeFromEvent, snapToGrid, useSelectedItems, useTempoMap, useTimelineView } from "@/timeline";
import { cmd } from "@/transport";
import { clipMenu, selectTrackEntity, trackMenu } from "./actions";
import { smallClipAt, splitSmallClips } from "./smallClips";
import { onClipPointerDown } from "./clipDrag";
import { SmallClipsLayer } from "./SmallClipsLayer";
import { hasClipboard, pasteClips } from "./clipboard";
import { onLaneInsertPointerDown, type InsertSpan } from "./clipInsert";
import { ClipView } from "./ClipView";
import { colorCss, groupSummaryKey, inkOn } from "./helpers";
import { sendEdit, useArrangement } from "./context";
import { beatsCss, useSettledZoom, widthCss } from "./laneGeometry";
import { laneItems } from "./laneItems";
import { DraftRow } from "./newTrack";
import { midiTarget } from "@/features/midi-learn/targets";
import { HeaderVolume } from "./HeaderVolume";
import { onTrackHeaderPointerDown } from "./trackDrag";
import { TRACK_HEIGHT_STEP, type Row } from "./layout";
import { arrangementView, useArrangementUi, type PendingImport } from "./uiStore";

/** Pointer tolerance around painted small clips (px): very thin ones stay clickable. */
const SMALL_SLOP_PX = 3;
const EMPTY_CLIPS: Readonly<Record<ClipId, Clip>> = {};
const INDENT_PX = 12;

// Rows stack in flow and nothing below reads `y`: rows that only move (automation lanes
// animating above them) don't re-render.
export const TrackRow = memo(function TrackRow({ row }: { row: Row }) {
  if (row.draft) return <DraftRow row={row} />;
  return <RealTrackRow row={row} />;
}, (a, b) => sameRowExceptY(a.row, b.row));

function sameRowExceptY(a: Row, b: Row): boolean {
  return a.track === b.track && a.draft === b.draft && a.depth === b.depth && a.laneHeight === b.laneHeight && a.height === b.height;
}

function RealTrackRow({ row }: { row: Row }) {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const grid = useArrangementUi((s) => s.grid);
  // The lane part doesn't depend on the automation height (animated per frame).
  const { track, depth, laneHeight } = row;
  const mainRow = useMemo(() => row, [track, depth, laneHeight]); // eslint-disable-line react-hooks/exhaustive-deps
  const main = useMemo(
    () => (
      <div className="eth-arr-row__main" style={{ height: laneHeight }}>
        <TrackHeader row={mainRow} />
        <ResizeHandle row={mainRow} />
        {track.kind === "Group" ? <GroupLane track={track} /> : <TrackLane track={track} />}
      </div>
    ),
    [mainRow, track, laneHeight],
  );
  return (
    <div className="eth-arr-row" style={{ height: row.height }} data-track={row.track.id}>
      {main}
      {/* Automation slot: its height is fed to `layoutRows` via `useAutomationHeight`. */}
      <div className="eth-arr-row__automation" data-slot="automation" data-track={row.track.id}>
        <TrackAutomationLanes trackId={row.track.id} view={arrangementView} headerWidth={headerWidth} grid={grid} />
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
  // v0.2 (`groups-buses`): VCA tracks; the node refines the icon.
  Vca: <Folder />,
};

/**
 * A round badge in the track color with the track type's icon, in dark or light ink,
 * whichever contrasts more with the color (`inkOn`).
 */
function TrackBadge({ kind, color }: { kind: Track["kind"]; color: Color }) {
  return (
    <span className="eth-arr-header__badge" aria-hidden title={`${kind} track`} style={{ color: inkOn(color) }}>
      {TRACK_ICONS[kind]}
    </span>
  );
}

function TrackHeader({ row }: { row: Row }) {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const { track, depth } = row;
  const ctx = useArrangement();
  const { transport } = ctx;
  const dragging = useArrangementUi((s) => s.trackDrag?.track === track.id);
  const dropInto = useArrangementUi((s) => s.trackDrag?.into === track.id);
  // Highlighted only as the arrangement's selected entity (not while one of its clips is).
  const selected = useArrangementUi((s) => s.selectedTracks.has(track.id));
  const armed = useProjectStore((s) => s.armedTracks.includes(track.id));
  const folded = useArrangementUi((s) => s.folded.has(track.id));
  const [renaming, setRenaming] = useState(false);
  const { mute, solo } = track.mixer;
  const canArm = track.kind === "Audio" || track.kind === "Midi";
  const stop = (e: MouseEvent) => e.stopPropagation();

  return (
    <div
      className={clsx(
        "eth-arr-header",
        selected && "eth-arr-header--selected",
        dragging && "eth-arr-header--dragging",
        dropInto && "eth-arr-header--drop-into",
        `eth-arr-header--${track.kind.toLowerCase()}`,
      )}
      style={{
        width: headerWidth,
        paddingLeft: 14 + depth * INDENT_PX,
        ["--eth-track-color" as string]: colorCss(track.color),
        ["--eth-track-depth" as string]: depth,
      }}
      onPointerDown={(e) => {
        stop(e);
        if (!renaming) onTrackHeaderPointerDown(e, track, ctx);
      }}
      onClick={(e) => selectTrackEntity(track.id, selectModeFromEvent(e))}
      onContextMenu={(e) => openContextMenu(e, trackMenu(transport, track))}
      role="group"
      aria-label={`${track.name} track`}
    >
      {track.kind === "Group" ? (
        <button
          type="button"
          className="eth-arr-header__fold"
          aria-expanded={!folded}
          aria-label={folded ? `Unfold ${track.name}` : `Fold ${track.name}`}
          onClick={(e) => {
            stop(e);
            useArrangementUi.getState().toggleFold(track.id);
          }}
        >
          {folded ? <ChevronRight /> : <ChevronDown />}
        </button>
      ) : null}
      <TrackBadge kind={track.kind} color={track.color} />
      {renaming ? (
        <TrackNameInput
          name={track.name}
          onDone={(name) => {
            setRenaming(false);
            if (name !== null && name.trim() && name.trim() !== track.name) {
              void sendEdit(transport, cmd("Track", { type: "Rename", id: track.id, name: name.trim() }));
            }
          }}
        />
      ) : (
        <span
          className="eth-arr-header__name"
          title={`${track.name} (double-click to rename)`}
          onDoubleClick={(e) => {
            stop(e);
            setRenaming(true);
          }}
        >
          {track.name}
        </span>
      )}
      <HeaderVolume track={track} />
      <span className="eth-arr-header__buttons" onClick={stop}>
        <button
          type="button"
          className="eth-arr-header__toggle eth-arr-header__mute"
          {...midiTarget({ type: "TrackMute", track: track.id })}
          aria-pressed={mute}
          aria-label={`Mute ${track.name}`}
          title={mute ? "Unmute" : "Mute"}
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
            aria-label={`Solo ${track.name}`}
            title="Solo (Ctrl/Cmd-click to add to the soloed tracks)"
            onClick={(e) =>
              void sendEdit(
                transport,
                cmd("Mixer", { type: "SetSolo", track: track.id, solo: !solo, exclusive: !solo && !(e.ctrlKey || e.metaKey) }),
              )
            }
          >
            <Headphones />
          </button>
        )}
        {canArm && (
          <button
            type="button"
            className="eth-arr-header__toggle eth-arr-header__arm"
            {...midiTarget({ type: "TrackArm", track: track.id })}
            aria-pressed={armed}
            aria-label={`Arm ${track.name}`}
            title="Record arm (Ctrl/Cmd-click to arm several tracks)"
            onClick={(e) =>
              void transport
                .send(
                  cmd("Recording", { type: "Arm", track: track.id, armed: !armed, exclusive: !(e.ctrlKey || e.metaKey) }),
                )
                // Arming an audio track with no input device open: offer to choose one.
                .then(() => {
                  if (!armed && track.kind === "Audio") void promptForInputIfNone(transport);
                })
                .catch((err: unknown) => console.warn("arm failed", err))
            }
          >
            <Circle />
          </button>
        )}
        <AutomationToggle track={track} />
      </span>
      <HeaderMeter track={track.id} />
    </div>
  );
}

/** Inline rename field: Enter or blur commits, Escape cancels (`onDone(null)`). */
function TrackNameInput({ name, onDone }: { name: string; onDone: (name: string | null) => void }) {
  const [value, setValue] = useState(name);
  const done = useRef(false);
  const finish = (v: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(v);
  };
  return (
    <input
      className="eth-arr-header__rename"
      aria-label="Track name"
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
function HeaderMeter({ track }: { track: TrackId }) {
  const meter = useTrackMeter(track);
  const level = meter ? meterPosition(Math.max(meter.peak[0], meter.peak[1])) : 0;
  return (
    <span className="eth-arr-header__meter" aria-hidden>
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
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    const startY = e.clientY;
    const startH = row.laneHeight;
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
      style={{ top: row.laneHeight - 3, width: headerWidth }}
      onPointerDown={onPointerDown}
      onDoubleClick={(e) => {
        e.stopPropagation();
        useArrangementUi.getState().setHeight(row.track.id, null);
      }}
      role="separator"
      aria-orientation="horizontal"
      aria-label={`Resize ${row.track.name}`}
      title="Drag to resize the track (double-click to reset)"
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
  // Zoom: the settled one (see laneGeometry.ts); positions follow the live zoom via --ppb.
  const pxPerBeat = useSettledZoom(arrangementView);
  const width = useTimelineView(arrangementView, (s) => s.widthPx) || 4000;
  const span = width / pxPerBeat;
  const step = useTimelineView(arrangementView, (s) => Math.floor(s.scrollBeats / span));
  return useMemo(() => {
    const origin = Math.max(0, (step - 1) * span);
    return {
      vp: { pxPerBeat, scrollBeats: origin },
      // One extra span on the right: until the zoom settles, zooming out shows up to
      // MAX_DRIFT spans.
      visible: { start: origin, end: (step + 3) * span },
    };
  }, [pxPerBeat, step, span]);
}

/**
 * Positions lane content (laid out in beats from `origin`, see laneGeometry.ts) at the live
 * zoom and scroll, outside React: writes `--ppb` and the scroll transform on every change.
 */
function LaneLayer({ origin, children }: { origin: Beats; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const view = arrangementView;
    const apply = () => {
      const s = view.getState();
      el.style.setProperty("--ppb", `${s.pxPerBeat}px`);
      el.style.transform = `translateX(${(origin - s.scrollBeats) * s.pxPerBeat}px)`;
    };
    apply();
    return view.subscribe(apply);
  }, [origin]);
  return (
    <div className="eth-arr-lane__layer" ref={ref}>
      {children}
    </div>
  );
}

/** The clip lane of an audio or MIDI (or return/master: empty) track. */
function TrackLane({ track }: { track: Track }) {
  const ctx = useArrangement();
  const clips = useProjectStore((s) => s.project?.clips ?? EMPTY_CLIPS);
  const preview = useArrangementUi((s) => s.preview);
  const dropHint = useArrangementUi((s) => (s.dropHint?.track === track.id ? s.dropHint.at : null));
  const imports = useArrangementUi((s) => s.imports);
  const tempo = useTempoMap();
  const { vp, visible } = useLaneView();
  const items = useMemo(() => laneItems(clips, track.id, preview), [clips, track.id, preview]);
  // Clips too small to be elements are painted on one canvas per lane (smallClips.ts).
  const { singles, small } = useMemo(() => splitSmallClips(items, vp.pxPerBeat), [items, vp.pxPerBeat]);
  const selectedClips = useSelectedItems("clip");

  const [insert, setInsert] = useState<InsertSpan | null>(null);

  /** The painted clip under the pointer (with a few px of tolerance: they can be very thin). */
  const smallUnder = (e: { clientX: number; currentTarget: HTMLElement }) => {
    if (!small.length) return null;
    const s = arrangementView.getState();
    const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
    return smallClipAt(small, pxToBeats(x, s), SMALL_SLOP_PX / s.pxPerBeat);
  };

  return (
    <div
      className={clsx("eth-arr-lane", track.kind !== "Audio" && track.kind !== "Midi" && "eth-arr-lane--no-clips")}
      data-lane={track.id}
      onPointerDown={(e) => {
        // A painted clip behaves like a clip's title bar (select, drag, cmd-copy).
        const it = e.button === 0 ? smallUnder(e) : null;
        if (it) {
          onClipPointerDown(e, it.clip, ctx);
          return;
        }
        if (track.kind === "Midi") onLaneInsertPointerDown(e, track.id, ctx, setInsert);
      }}
      onDoubleClick={(e) => {
        const it = smallUnder(e);
        if (it) useEditorStore.getState().openClip(it.clip.id);
      }}
      onContextMenu={(e) => {
        const it = smallUnder(e);
        if (it) {
          openContextMenu(e, clipMenu(ctx.transport, it.clip));
          return;
        }
        // Empty space (or a clip's body, which lets clicks through): paste here.
        const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
        const s = arrangementView.getState();
        const step = resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
        const at = Math.max(0, snapToGrid(pxToBeats(x, s), e.altKey ? null : step, tempo, "floor"));
        openContextMenu(e, [
          { label: "Paste", shortcut: `${MOD_KEY}V`, disabled: !hasClipboard(), onSelect: () => void pasteClips(ctx.transport, at, track.id) },
          // collab-social: pin a note here (hidden while "Hide users and notes" is on).
          ...withSeparator(leaveNoteEntries({ kind: "arranger" }, e)),
        ]);
      }}
    >
      <LaneLayer origin={vp.scrollBeats}>
      {small.length > 0 && (
        <SmallClipsLayer
          items={small}
          origin={vp.scrollBeats}
          visible={visible}
          pxPerBeat={vp.pxPerBeat}
          trackColor={track.color}
          selected={selectedClips}
        />
      )}
      {singles.map((it) =>
        it.bounds.start + it.bounds.length < visible.start || it.bounds.start > visible.end ? null : (
          <ClipView
            key={(it.ghost ? "ghost:" : "") + it.clip.id}
            clip={it.clip}
            bounds={it.bounds}
            trackColor={track.color}
            vp={vp}
            visible={visible}
            tempo={tempo}
            dragging={it.dragging}
            ghost={it.ghost}
          />
        ),
      )}
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
      {dropHint !== null && (
        <div className="eth-arr-lane__drop-hint" style={{ left: beatsCss(dropHint - vp.scrollBeats) }} />
      )}
      {imports.map((i) =>
        i.track === track.id ? (
          <ImportPlaceholder key={i.id} item={i} style={{ left: beatsCss(i.at - vp.scrollBeats) }} />
        ) : null,
      )}
      {(track.kind === "Audio" || track.kind === "Midi") && (
        <LiveRecordLane
          transport={ctx.transport}
          track={track.id}
          color={track.color}
          origin={vp.scrollBeats}
          visible={visible}
          pxPerBeat={vp.pxPerBeat}
        />
      )}
      </LaneLayer>
    </div>
  );
}

/** A browser drop still importing (or failed), shown where the clip will go. */
export function ImportPlaceholder({ item, style }: { item: PendingImport; style?: CSSProperties }) {
  const text = item.error
    ? `Import failed: ${item.name}`
    : `Importing ${item.name}…${item.progress !== null ? ` ${Math.round(item.progress * 100)}%` : ""}`;
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

/** Group lane: a summary of the clips of every track inside the group. */
function GroupLane({ track }: { track: Track }) {
  const summary = useProjectStore((s) => (s.project ? groupSummaryKey(s.project.tracks, s.project.clips, track.id) : ""));
  const { vp } = useLaneView();
  const spans = useMemo(() => (summary ? summary.split(";").map((p) => p.split(",").map(Number) as [number, number]) : []), [summary]);
  return (
    <div className="eth-arr-lane eth-arr-lane--group" data-lane={track.id} style={{ ["--eth-track-color" as string]: colorCss(track.color) }}>
      <LaneLayer origin={vp.scrollBeats}>
        {spans.map(([s, l], i) => (
          <div
            key={i}
            className="eth-arr-lane__summary"
            style={{ left: beatsCss(s - vp.scrollBeats), width: widthCss(l, 1) }}
          />
        ))}
      </LaneLayer>
    </div>
  );
}

