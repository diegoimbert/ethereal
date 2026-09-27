import "./clipEditing.css";
import clsx from "clsx";
import { useContext, useRef, useState, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Beats, Marker } from "@/generated";
import { IconButton, openContextMenu, setDragCursor, TextInput, type ContextMenuEntry } from "@/kit";
import { playheadStore, useProjectStore } from "@/state";
import { pxToBeats, resolveGrid, snapToGrid, TempoMap, useViewport } from "@/timeline";
import { cmd, newId, TransportContext, type EngineTransport } from "@/transport";
import { LaneGesture } from "@/features/automation/gesture";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { colorCss } from "@/features/arrangement/helpers";
import { sendEdit } from "./clipEditing";

const DRAG_THRESHOLD_PX = 3;

/** Markers sorted by position (then id, for a stable order). */
function sortedMarkers(markers: Readonly<Record<string, Marker>>): Marker[] {
  return Object.values(markers).sort((a, b) => a.position - b.position || a.id.localeCompare(b.id));
}

/** Arrangement grid snap (alt/option bypasses it), as for clips. */
function snap(beats: Beats, bypass: boolean): Beats {
  const project = useProjectStore.getState().project;
  if (bypass || !project) return Math.max(0, beats);
  const tempo = TempoMap.fromProject(project);
  const s = arrangementView.getState();
  const step = resolveGrid(useArrangementUi.getState().grid, s.pxPerBeat, tempo.signatureAt(s.scrollBeats));
  return Math.max(0, snapToGrid(beats, step, tempo));
}

const addMarkerCommand = (position: Beats) =>
  cmd("Marker", { type: "Add", id: newId(), position: Math.max(0, position), name: null, color: null });

const locate = (transport: EngineTransport, position: Beats) =>
  transport.send(cmd("Transport", { type: "Locate", position })).catch((e) => console.warn("[ethereal] locate failed", e));

/**
 * Arrangement markers, above the arrangement and aligned with its timeline: click a marker
 * to jump there, drag it to move it (snapped to the grid; alt/option bypasses the grid),
 * double-click it to rename it, right-click for more. Double-click the empty lane (or use
 * "+", at the playhead) to add a marker.
 */
export function MarkerLane() {
  const ctx = useContext(TransportContext);
  if (!ctx) return null;
  return <ConnectedMarkerLane transport={ctx.transport} />;
}

function ConnectedMarkerLane({ transport }: { transport: EngineTransport }) {
  const vp = useViewport(arrangementView);
  const markers = useProjectStore(useShallow((s) => (s.project ? sortedMarkers(s.project.markers) : [])));
  const hasProject = useProjectStore((s) => !!s.project);
  const laneRef = useRef<HTMLDivElement>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  if (!hasProject) return null;

  const beatAt = (clientX: number) => pxToBeats(clientX - (laneRef.current?.getBoundingClientRect().left ?? 0), arrangementView.getState());

  const addAt = (position: Beats) => void sendEdit(transport, addMarkerCommand(position));

  const onMarkerPointerDown = (e: ReactPointerEvent<HTMLElement>, m: Marker) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    e.preventDefault();
    const x0 = e.clientX;
    const { pxPerBeat } = arrangementView.getState();
    let gesture: LaneGesture | null = null;
    const move = (ev: PointerEvent) => {
      if (!gesture && Math.abs(ev.clientX - x0) < DRAG_THRESHOLD_PX) return;
      if (!gesture) {
        gesture = new LaneGesture(transport);
        setDragging(m.id);
        setDragCursor("grabbing");
      }
      const position = snap(m.position + (ev.clientX - x0) / pxPerBeat, ev.altKey);
      gesture.update(cmd("Marker", { type: "Move", id: m.id, position }));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      if (gesture) {
        gesture.end();
        setDragging(null);
        setDragCursor(null);
      } else {
        void locate(transport, m.position);
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const markerMenu = (m: Marker): ContextMenuEntry[] => [
    { label: "Jump to Marker", onSelect: () => void locate(transport, m.position) },
    { label: "Rename", onSelect: () => setRenaming(m.id) },
    "separator",
    { label: "Delete Marker", danger: true, onSelect: () => void sendEdit(transport, cmd("Marker", { type: "Remove", ids: [m.id] })) },
  ];

  const laneMenu = (e: MouseEvent): ContextMenuEntry[] => {
    const at = snap(beatAt(e.clientX), e.altKey);
    return [{ label: "Add Marker Here", onSelect: () => addAt(at) }];
  };

  const renamed = markers.find((m) => m.id === renaming);

  return (
    <div className="eth-markers" data-feature="markers" data-testid="marker-lane">
      <div className="eth-markers__corner" style={{ width: headerWidth }}>
        <span>Markers</span>
        <IconButton
          label="Add marker at playhead"
          icon="+"
          size="sm"
          tone="ghost"
          onClick={() => addAt(playheadStore.getPlayhead()?.transport.position ?? 0)}
        />
      </div>
      <div
        ref={laneRef}
        className="eth-markers__lane"
        data-testid="marker-lane-area"
        onDoubleClick={(e) => addAt(snap(beatAt(e.clientX), e.altKey))}
        onContextMenu={(e) => openContextMenu(e, laneMenu(e))}
      >
        {markers.map((m) => (
          <div
            key={m.id}
            className={clsx("eth-markers__marker", dragging === m.id && "eth-markers__marker--dragging")}
            style={{
              left: (m.position - vp.scrollBeats) * vp.pxPerBeat,
              ...(m.color !== null ? { ["--eth-marker-color" as string]: colorCss(m.color) } : {}),
            }}
            data-marker-id={m.id}
            data-testid="marker"
            role="button"
            aria-label={`Marker ${m.name}`}
            title={`${m.name} (click to jump, drag to move, double-click to rename)`}
            onPointerDown={(e) => onMarkerPointerDown(e, m)}
            onDoubleClick={(e) => {
              e.stopPropagation();
              setRenaming(m.id);
            }}
            onContextMenu={(e) => openContextMenu(e, markerMenu(m))}
          >
            {m.name}
          </div>
        ))}
        {renamed && (
          <RenameField
            key={renamed.id}
            marker={renamed}
            left={(renamed.position - vp.scrollBeats) * vp.pxPerBeat}
            onDone={(name) => {
              setRenaming(null);
              const next = name?.trim();
              if (next && next !== renamed.name) void sendEdit(transport, cmd("Marker", { type: "Rename", id: renamed.id, name: next }));
            }}
          />
        )}
      </div>
    </div>
  );
}

function RenameField({ marker, left, onDone }: { marker: Marker; left: number; onDone: (name: string | null) => void }) {
  const done = useRef(false);
  const finish = (name: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(name);
  };
  return (
    <div className="eth-markers__rename" style={{ left }}>
      <TextInput
        size="sm"
        autoFocus
        defaultValue={marker.name}
        aria-label="Marker name"
        onFocus={(e) => e.currentTarget.select()}
        onKeyDown={(e) => {
          if (e.key === "Enter") finish(e.currentTarget.value);
          else if (e.key === "Escape") finish(null);
          e.stopPropagation();
        }}
        onBlur={(e) => finish(e.currentTarget.value)}
      />
    </div>
  );
}
