import clsx from "clsx";
import { memo, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type PointerEvent } from "react";
import { ChevronDown, ChevronRight, Circle, Headphones, Volume2, VolumeX } from "lucide-react";
import type { Clip, ClipId, Track, TrackId } from "@/generated";
import { TrackAutomationLanes } from "@/features/automation";
import { MOD_KEY, meterPosition, openContextMenu, setDragCursor } from "@/kit";
import { useProjectStore, useTrackMeter } from "@/state";
import { pxToBeats, resolveGrid, snapToGrid, useTempoMap, useTimelineView, useViewport, visibleRange } from "@/timeline";
import { cmd } from "@/transport";
import { selectTrackEntity, trackMenu } from "./actions";
import { hasClipboard, pasteClips } from "./clipboard";
import { onLaneInsertPointerDown, type InsertSpan } from "./clipInsert";
import { ClipView } from "./ClipView";
import { colorCss, groupSummaryKey } from "./helpers";
import { sendEdit, useArrangement } from "./context";
import { laneItems } from "./laneItems";
import { onTrackHeaderPointerDown } from "./trackDrag";
import { HEADER_WIDTH, TRACK_HEIGHT_STEP, type Row } from "./layout";
import { arrangementView, useArrangementUi, type PendingImport } from "./uiStore";

const EMPTY_CLIPS: Readonly<Record<ClipId, Clip>> = {};
const INDENT_PX = 12;

export const TrackRow = memo(function TrackRow({ row }: { row: Row }) {
  const grid = useArrangementUi((s) => s.grid);
  return (
    <div className="eth-arr-row" style={{ height: row.height }} data-track={row.track.id}>
      <div className="eth-arr-row__main" style={{ height: row.laneHeight }}>
        <TrackHeader row={row} />
        <ResizeHandle row={row} />
        {row.track.kind === "Group" ? <GroupLane track={row.track} /> : <TrackLane track={row.track} />}
      </div>
      {/* Automation slot: its height is fed to `layoutRows` via `useAutomationHeight`. */}
      <div className="eth-arr-row__automation" data-slot="automation" data-track={row.track.id}>
        <TrackAutomationLanes trackId={row.track.id} view={arrangementView} headerWidth={HEADER_WIDTH} grid={grid} />
      </div>
    </div>
  );
});

function TrackHeader({ row }: { row: Row }) {
  const { track, depth } = row;
  const ctx = useArrangement();
  const { transport } = ctx;
  const dragging = useArrangementUi((s) => s.trackDrag?.track === track.id);
  const dropInto = useArrangementUi((s) => s.trackDrag?.into === track.id);
  // Highlighted only as the arrangement's selected entity (not while one of its clips is).
  const selected = useArrangementUi((s) => s.trackFocus === track.id);
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
      style={{ width: HEADER_WIDTH, paddingLeft: 8 + depth * INDENT_PX, ["--eth-track-color" as string]: colorCss(track.color) }}
      onPointerDown={(e) => {
        stop(e);
        if (!renaming) onTrackHeaderPointerDown(e, track, ctx);
      }}
      onClick={() => selectTrackEntity(track.id)}
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
      <span className="eth-arr-header__buttons" onClick={stop}>
        <button
          type="button"
          className="eth-arr-header__toggle eth-arr-header__mute"
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
            aria-pressed={armed}
            aria-label={`Arm ${track.name}`}
            title="Record arm (Ctrl/Cmd-click to arm several tracks)"
            onClick={(e) =>
              void transport
                .send(
                  cmd("Recording", { type: "Arm", track: track.id, armed: !armed, exclusive: !(e.ctrlKey || e.metaKey) }),
                )
                .catch((err: unknown) => console.warn("arm failed", err))
            }
          >
            <Circle />
          </button>
        )}
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
      style={{ top: row.laneHeight - 3, width: HEADER_WIDTH }}
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

/** Visible beat range of the arrangement view (re-renders on scroll/zoom/resize). */
function useVisible() {
  const vp = useViewport(arrangementView);
  const width = useTimelineView(arrangementView, (s) => s.widthPx);
  const visible = useMemo(() => {
    const r = visibleRange(vp, width || 4000);
    return { start: r.start, end: r.end };
  }, [vp, width]);
  return { vp, visible };
}

/** The clip lane of an audio or MIDI (or return/master: empty) track. */
function TrackLane({ track }: { track: Track }) {
  const ctx = useArrangement();
  const clips = useProjectStore((s) => s.project?.clips ?? EMPTY_CLIPS);
  const preview = useArrangementUi((s) => s.preview);
  const dropHint = useArrangementUi((s) => (s.dropHint?.track === track.id ? s.dropHint.at : null));
  const imports = useArrangementUi((s) => s.imports);
  const tempo = useTempoMap();
  const { vp, visible } = useVisible();
  const items = useMemo(() => laneItems(clips, track.id, preview), [clips, track.id, preview]);

  const [insert, setInsert] = useState<InsertSpan | null>(null);

  return (
    <div
      className={clsx("eth-arr-lane", track.kind !== "Audio" && track.kind !== "Midi" && "eth-arr-lane--no-clips")}
      data-lane={track.id}
      onPointerDown={track.kind === "Midi" ? (e) => onLaneInsertPointerDown(e, track.id, ctx, setInsert) : undefined}
      onContextMenu={(e) => {
        // Empty space (or a clip's body, which lets clicks through): paste here.
        const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
        const s = arrangementView.getState();
        const step = resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
        const at = Math.max(0, snapToGrid(pxToBeats(x, s), e.altKey ? null : step, tempo, "floor"));
        openContextMenu(e, [
          { label: "Paste", shortcut: `${MOD_KEY}V`, disabled: !hasClipboard(), onSelect: () => void pasteClips(ctx.transport, at, track.id) },
        ]);
      }}
    >
      {items.map((it) =>
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
            left: (insert.start - vp.scrollBeats) * vp.pxPerBeat,
            width: insert.length * vp.pxPerBeat,
            ["--eth-clip-color" as string]: colorCss(track.color),
          }}
        />
      )}
      {dropHint !== null && (
        <div className="eth-arr-lane__drop-hint" style={{ left: (dropHint - vp.scrollBeats) * vp.pxPerBeat }} />
      )}
      {imports.map((i) =>
        i.track === track.id ? (
          <ImportPlaceholder key={i.id} item={i} style={{ left: (i.at - vp.scrollBeats) * vp.pxPerBeat }} />
        ) : null,
      )}
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
  const { vp } = useVisible();
  const spans = useMemo(() => (summary ? summary.split(";").map((p) => p.split(",").map(Number) as [number, number]) : []), [summary]);
  return (
    <div className="eth-arr-lane eth-arr-lane--group" data-lane={track.id} style={{ ["--eth-track-color" as string]: colorCss(track.color) }}>
      {spans.map(([s, l], i) => (
        <div
          key={i}
          className="eth-arr-lane__summary"
          style={{ left: (s - vp.scrollBeats) * vp.pxPerBeat, width: Math.max(1, l * vp.pxPerBeat) }}
        />
      ))}
    </div>
  );
}

