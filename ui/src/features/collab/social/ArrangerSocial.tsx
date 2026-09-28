// collab-social over the arrangement (docs/COLLAB.md §12.2-§12.3), mounted next to
// presence-v2's `PresenceLayer` with the same props: pinned notes (dots, pointer events on
// the dots only) and the peers' playheads (pointer-events: none). Song coordinates map to
// this user's own layout with presence-v2's `coords.ts`.
import "./social.css";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type RefObject } from "react";
import type { NotePosition, SiteId } from "@/generated";
import { useProjectStore } from "@/state";
import { useTempoMap } from "@/timeline";
import { DROP_AREA_HEIGHT } from "@/features/arrangement/layout";
import { arrangementView } from "@/features/arrangement/uiStore";
import { activeHost } from "../listen/store";
import { useListenStore } from "../listen";
import { beatsToX, clampToBox, screenToSong, songToScreen, type Box } from "../presence/coords";
import type { PresenceLayerProps } from "../presence/PresenceLayer";
import { measure, parentOf, type Geometry } from "./arrangerGeometry";
import { initials, peerColor, useCollabStore, useHideOthers } from "../store";
import { DraftDot, NoteDot } from "./notes/NoteDot";
import { notesOn, registerNoteSurface, useNotesUi, type NoteSurface } from "./notes/notesStore";
import { extrapolate, SampleTracker } from "./playheads/extrapolate";

const ARRANGER: NoteSurface = { kind: "arranger" };
type LayerRef = RefObject<PresenceLayerProps>;

function geometryOf(layer: PresenceLayerProps): Geometry | null {
  const root = layer.rootRef.current;
  const scroll = layer.scrollRef.current;
  return root && scroll ? measure(root, scroll, layer.rows, layer.masterRow) : null;
}

/** The song position under a client point (as a note position), `null` outside the lanes. */
function arrangerPositionAt(layer: PresenceLayerProps, clientX: number, clientY: number): NotePosition | null {
  const root = layer.rootRef.current;
  const g = geometryOf(layer);
  if (!root || !g) return null;
  const r = root.getBoundingClientRect();
  const x = clientX - r.left;
  const y = clientY - r.top;
  const bottom = g.masterBox?.y1 ?? g.scrollBox.y1;
  if (y < g.rulerTop || y >= bottom || x >= g.scrollBox.x1) return null;
  if (y < g.scrollBox.y0) return { ...screenToSong(x, y, [], g.lanes), track: null, y: 0 };
  const inMaster = !!g.masterBox && y >= g.masterBox.y0;
  const band = g.rows.filter((row) => (row.track === g.masterTrack) === inMaster);
  const p = screenToSong(x, y, band, g.lanes, inMaster ? undefined : g.free, DROP_AREA_HEIGHT);
  return { beats: p.beats, track: p.track, y: p.y };
}

/** Screen point of a song position (a note whose track is gone sits on the ruler row). */
function place(position: NotePosition, g: Geometry): { x: number; y: number; box: Box } {
  const p = { beats: position.beats, track: position.track, y: position.y };
  const hit = songToScreen(p, g.rows, g.lanes, parentOf, g.rulerY, g.free, DROP_AREA_HEIGHT);
  const at = hit ?? { x: beatsToX(p.beats, g.lanes), y: g.rulerY };
  const onRuler = !hit || (p.track === null && p.y <= 0);
  const box = onRuler
    ? { ...g.scrollBox, y0: g.rulerTop, y1: g.scrollBox.y0 }
    : p.track !== null && p.track === g.masterTrack && g.masterBox
      ? g.masterBox
      : g.scrollBox;
  return { x: at.x, y: at.y, box };
}

/** Re-render on scroll, zoom, resize and layout changes (rAF-coalesced). */
function useLayoutTick(layer: LayerRef, rows: unknown): number {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let frame = 0;
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(() => ((frame = 0), setTick((t) => t + 1)));
    };
    const scroll = layer.current.scrollRef.current;
    const root = layer.current.rootRef.current;
    const offView = arrangementView.subscribe(schedule);
    scroll?.addEventListener("scroll", schedule, { passive: true });
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(schedule) : null;
    if (root) ro?.observe(root);
    window.addEventListener("resize", schedule);
    schedule();
    return () => {
      offView();
      scroll?.removeEventListener("scroll", schedule);
      ro?.disconnect();
      window.removeEventListener("resize", schedule);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [layer, rows]);
  return tick;
}

export function ArrangerSocialLayer(props: PresenceLayerProps) {
  const layer = useRef(props);
  useLayoutEffect(() => {
    layer.current = props;
  });
  const hide = useHideOthers();
  const online = useCollabStore((s) => s.status.type === "Online");
  // "Leave a note" maps the right-click point with this layout.
  useEffect(() => registerNoteSurface(ARRANGER, (x, y) => arrangerPositionAt(layer.current, x, y)), []);
  return (
    <>
      {!hide && <ArrangerNotes layer={layer} rows={props.rows} />}
      {online && !hide && <PeerPlayheads layer={layer} />}
    </>
  );
}

function ArrangerNotes({ layer, rows }: { layer: LayerRef; rows: unknown }) {
  const all = useProjectStore((s) => s.project?.pinned_notes);
  const notes = useMemo(() => notesOn(all, ARRANGER), [all]);
  const draft = useNotesUi((s) => (s.draft?.surface.kind === "arranger" ? s.draft : null));
  const status = useCollabStore((s) => s.status);
  const me = status.type === "Online" ? status.site : null;
  useLayoutTick(layer, rows);
  const g = geometryOf(layer.current);
  if (!g || (notes.length === 0 && !draft)) return null;
  const shown = (position: NotePosition) => {
    const { x, y, box } = place(position, g);
    const c = clampToBox(x, y, box);
    return c.edge === null ? { x, y } : null;
  };
  return (
    <div className="eth-notes-layer" data-testid="arranger-notes">
      {notes.map((n) => {
        const at = shown(n.position);
        return at && <NoteDot key={n.id} note={n} surface={ARRANGER} x={at.x} y={at.y} me={me} />;
      })}
      {draft &&
        (() => {
          const at = shown(draft.position);
          return at && <DraftDot x={at.x} y={at.y} position={draft.position} color="var(--eth-color-accent)" />;
        })()}
    </div>
  );
}

/** One line + ruler cap per peer, in its colour, extrapolated per frame (§12.3). */
function PeerPlayheads({ layer }: { layer: LayerRef }) {
  const peers = useCollabStore((s) => s.peers);
  const listeningTo = useListenStore((s) => activeHost(s.listening));
  const tempo = useTempoMap();
  const tracker = useRef(new SampleTracker());
  const els = useRef(new Map<SiteId, HTMLDivElement>());
  // The host we listen to already drives our own playhead (stream-listen): not twice.
  const withTransport = peers.filter((p) => p.state.transport && p.site !== listeningTo);
  const sites = withTransport.map((p) => p.site).join(",");
  useEffect(() => {
    tracker.current.update(peers, performance.now());
  }, [peers]);
  const draw = useCallback(() => {
    const g = geometryOf(layer.current);
    const now = performance.now();
    for (const [site, el] of els.current) {
      const sample = tracker.current.get(site);
      if (!g || !sample) {
        el.dataset.hidden = "true";
        continue;
      }
      const beats = extrapolate(sample, now, tempo);
      const x = g.lanes.left + (beats - g.lanes.scrollBeats) * g.lanes.pxPerBeat;
      const visible = x >= g.scrollBox.x0 && x <= g.scrollBox.x1;
      el.dataset.hidden = String(!visible);
      el.dataset.playing = String(sample.transport.playing);
      el.dataset.beats = beats.toFixed(3);
      el.style.transform = `translateX(${x}px)`;
      el.style.top = `${g.rulerTop}px`;
      el.style.height = `${(g.masterBox?.y1 ?? g.scrollBox.y1) - g.rulerTop}px`;
    }
  }, [layer, tempo]);
  useEffect(() => {
    if (!sites) return;
    let frame = 0;
    const loop = () => {
      frame = requestAnimationFrame(loop);
      draw();
    };
    loop();
    return () => cancelAnimationFrame(frame);
  }, [sites, draw]);
  if (withTransport.length === 0) return null;
  return (
    <div className="eth-peer-playheads" aria-hidden data-testid="peer-playheads">
      {withTransport.map((p) => (
        <div
          key={p.site}
          ref={(el) => {
            if (el) els.current.set(p.site, el);
            else els.current.delete(p.site);
          }}
          className="eth-peer-playhead"
          data-testid="peer-playhead"
          data-peer={p.name || "Anonymous"}
          data-hidden="true"
          style={{ "--eth-collab-peer": peerColor(p.color) } as CSSProperties}
        >
          <span className="eth-peer-playhead__cap">{initials(p.name)}</span>
        </div>
      ))}
    </div>
  );
}
