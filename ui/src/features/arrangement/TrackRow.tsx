import clsx from "clsx";
import { memo, useMemo, type MouseEvent } from "react";
import type { Clip, ClipId, Track } from "@/generated";
import { Button } from "@/kit";
import { useProjectStore, useSelectionStore } from "@/state";
import { pxToBeats, useTempoMap, useTimelineView, useViewport, visibleRange } from "@/timeline";
import { cmd, newId } from "@/transport";
import { ClipView } from "./ClipView";
import { barAround, colorCss, groupSummaryKey } from "./helpers";
import { sendEdit, useArrangement } from "./context";
import { laneItems } from "./laneItems";
import { HEADER_WIDTH, type Row } from "./layout";
import { arrangementView, useArrangementUi } from "./uiStore";

const EMPTY_CLIPS: Readonly<Record<ClipId, Clip>> = {};
const INDENT_PX = 12;

export const TrackRow = memo(function TrackRow({ row }: { row: Row }) {
  return (
    <div className="eth-arr-row" style={{ height: row.height }} data-track={row.track.id}>
      <div className="eth-arr-row__main" style={{ height: row.laneHeight }}>
        <TrackHeader row={row} />
        {row.track.kind === "Group" ? <GroupLane track={row.track} /> : <TrackLane track={row.track} />}
      </div>
      {/*
        Automation slot: ui-automation mounts the track's automation lanes here (header part
        in the first HEADER_WIDTH px, lane part after it, same horizontal viewport as the
        clips: `arrangementView`). Its height must be reported to `layoutRows`.
      */}
      <div className="eth-arr-row__automation" data-slot="automation" data-track={row.track.id} />
    </div>
  );
});

function TrackHeader({ row }: { row: Row }) {
  const { track, depth } = row;
  const { transport } = useArrangement();
  const selected = useSelectionStore((s) => s.selectedTrack === track.id);
  const armed = useProjectStore((s) => s.armedTracks.includes(track.id));
  const folded = useArrangementUi((s) => s.folded.has(track.id));
  const { mute, solo } = track.mixer;
  const canArm = track.kind === "Audio" || track.kind === "Midi";
  const stop = (e: MouseEvent) => e.stopPropagation();

  return (
    <div
      className={clsx("eth-arr-header", selected && "eth-arr-header--selected", `eth-arr-header--${track.kind.toLowerCase()}`)}
      style={{ width: HEADER_WIDTH, paddingLeft: 4 + depth * INDENT_PX, ["--eth-track-color" as string]: colorCss(track.color) }}
      onPointerDown={stop}
      onClick={() => useSelectionStore.getState().selectTrack(track.id)}
      role="group"
      aria-label={`${track.name} track`}
    >
      <span className="eth-arr-header__color" />
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
          {folded ? "▸" : "▾"}
        </button>
      ) : null}
      <span className="eth-arr-header__name" title={track.name}>
        {track.name}
      </span>
      <span className="eth-arr-header__buttons" onClick={stop}>
        <Button
          size="sm"
          className="eth-arr-header__mute"
          active={mute}
          aria-label={`Mute ${track.name}`}
          onClick={() => void sendEdit(transport, cmd("Mixer", { type: "SetMute", track: track.id, mute: !mute }))}
        >
          M
        </Button>
        {track.kind !== "Master" && (
          <Button
            size="sm"
            className="eth-arr-header__solo"
            active={solo}
            aria-label={`Solo ${track.name}`}
            title="Solo (Ctrl/Cmd-click to add to the soloed tracks)"
            onClick={(e) =>
              void sendEdit(
                transport,
                cmd("Mixer", { type: "SetSolo", track: track.id, solo: !solo, exclusive: !solo && !(e.ctrlKey || e.metaKey) }),
              )
            }
          >
            S
          </Button>
        )}
        {canArm && (
          <Button
            size="sm"
            className="eth-arr-header__arm"
            active={armed}
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
            ●
          </Button>
        )}
      </span>
    </div>
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
  const { transport } = useArrangement();
  const clips = useProjectStore((s) => s.project?.clips ?? EMPTY_CLIPS);
  const preview = useArrangementUi((s) => s.preview);
  const dropHint = useArrangementUi((s) => (s.dropHint?.track === track.id ? s.dropHint.at : null));
  const tempo = useTempoMap();
  const { vp, visible } = useVisible();
  const items = useMemo(() => laneItems(clips, track.id, preview), [clips, track.id, preview]);

  const onDoubleClick = (e: MouseEvent<HTMLDivElement>) => {
    if (track.kind !== "Midi" || e.target !== e.currentTarget) return;
    const x = e.clientX - e.currentTarget.getBoundingClientRect().left;
    const at = pxToBeats(x, arrangementView.getState());
    const bar = barAround(tempo, at);
    void sendEdit(
      transport,
      cmd("Clip", { type: "CreateMidi", id: newId(), track: track.id, start: bar.start, length: bar.length, name: null }),
    );
  };

  return (
    <div
      className={clsx("eth-arr-lane", track.kind !== "Audio" && track.kind !== "Midi" && "eth-arr-lane--no-clips")}
      data-lane={track.id}
      onDoubleClick={onDoubleClick}
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
      {dropHint !== null && (
        <div className="eth-arr-lane__drop-hint" style={{ left: (dropHint - vp.scrollBeats) * vp.pxPerBeat }} />
      )}
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

