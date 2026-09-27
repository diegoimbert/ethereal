// Presence v2 in the arrangement (docs/COLLAB.md §8.3-§8.4), mounted once inside the
// arrangement root: publishes this user's pointer (song coordinates, rAF-coalesced, cleared
// on leave / blur / unmount) and viewport, applies a followed peer's viewport, and draws the
// peers' pointers (interpolated, in their colours, with an activity hint).
import "./presence.css";
import { useEffect, useLayoutEffect, useRef, type CSSProperties, type RefObject } from "react";
import type { ArrangerPointer, SiteId, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, useTransport } from "@/transport";
import { DROP_AREA_HEIGHT, type Row } from "@/features/arrangement/layout";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { peerColor, useCollabStore } from "../store";
import {
  clampToBox,
  screenToSong,
  scrollTopFor,
  songToScreen,
  viewportOf,
  type Box,
  type FreeSpace,
  type Lanes,
  type RowBox,
} from "./coords";
import { activityLabel, setFollowing, useLocalPresence } from "./local";
import { usePointerStore } from "./pointers";

const DRAFT = "__draft_track__";

export interface PresenceLayerProps {
  /** The arrangement root (`.eth-arr`, positioned): this layer covers it. */
  rootRef: RefObject<HTMLDivElement | null>;
  /** The vertically scrolling tracks (`.eth-arr__scroll`). */
  scrollRef: RefObject<HTMLDivElement | null>;
  /** Scrolling rows in content px, and the pinned master row (below the scroll area). */
  rows: ReadonlyArray<Row>;
  masterRow: Row | null;
}

const parentOf = (id: TrackId): TrackId | null | undefined => {
  const t = useProjectStore.getState().project?.tracks[id];
  return t ? t.parent : undefined;
};

interface Geometry {
  rows: RowBox[];
  lanes: Lanes;
  /** Visible scrolling lanes and the master lane (root px). */
  scrollBox: Box;
  masterBox: Box | null;
  masterTrack: TrackId | null;
  /** The ruler band (pointers over the ruler sit in its middle). */
  rulerTop: number;
  rulerY: number;
  /** Free space below the last scrolling track, down to the bottom of the view. */
  free: FreeSpace;
}

/** Rows and boxes in root px (from the DOM and the local layout). */
function measure(root: HTMLElement, scroll: HTMLElement, rows: ReadonlyArray<Row>, masterRow: Row | null): Geometry {
  const r = root.getBoundingClientRect();
  const s = scroll.getBoundingClientRect();
  const hw = useArrangementUi.getState().headerWidth;
  const v = arrangementView.getState();
  const offY = s.top - r.top - scroll.scrollTop;
  const x0 = s.left - r.left + hw;
  const x1 = s.left - r.left + scroll.clientWidth;
  const boxes: RowBox[] = rows.filter((row) => row.track.id !== DRAFT).map((row) => ({ track: row.track.id, top: offY + row.y, height: row.height }));
  const scrollBox = { x0, x1, y0: s.top - r.top, y1: s.top - r.top + scroll.clientHeight };
  const last = rows.at(-1);
  const free = { top: offY + (last ? last.y + last.height : 0), bottom: scrollBox.y1 };
  let masterBox: Box | null = null;
  const masterEl = masterRow ? root.querySelector<HTMLElement>('[data-testid="arrangement-master"]') : null;
  if (masterRow && masterEl) {
    const m = masterEl.getBoundingClientRect();
    masterBox = { x0, x1, y0: m.top - r.top, y1: m.bottom - r.top };
    boxes.push({ track: masterRow.track.id, top: m.top - r.top, height: m.height });
  }
  const ruler = root.querySelector<HTMLElement>(".eth-arr__top")?.getBoundingClientRect();
  const rulerTop = ruler ? ruler.top - r.top : scrollBox.y0;
  return {
    rows: boxes,
    lanes: { left: x0, pxPerBeat: v.pxPerBeat, scrollBeats: v.scrollBeats },
    scrollBox,
    masterBox,
    masterTrack: masterRow?.track.id ?? null,
    rulerTop,
    rulerY: ruler ? (ruler.top + ruler.bottom) / 2 - r.top : scrollBox.y0,
    free,
  };
}

/** Rows in content px (for the viewport: `top` from the first row). */
const contentRows = (rows: ReadonlyArray<Row>): RowBox[] =>
  rows.filter((row) => row.track.id !== DRAFT).map((row) => ({ track: row.track.id, top: row.y, height: row.height }));

const samePointer = (a: ArrangerPointer | null, b: ArrangerPointer | null) =>
  a === b || (!!a && !!b && a.track === b.track && Math.abs(a.beats - b.beats) < 1e-6 && Math.abs(a.y - b.y) < 1e-4);

export function PresenceLayer(props: PresenceLayerProps) {
  const online = useCollabStore((s) => s.status.type === "Online");
  const geo = useRef(props);
  useLayoutEffect(() => {
    geo.current = props;
  });
  usePublishPointer(geo, online);
  usePublishViewport(geo, props.rows);
  useFollow(geo, online);
  useEffect(() => {
    if (!online) {
      usePointerStore.getState().clear();
      setFollowing(null);
    }
  }, [online]);
  return online ? <PeerPointers layer={geo} /> : null;
}

type LayerRef = RefObject<PresenceLayerProps>;

function usePublishPointer(layer: LayerRef, online: boolean) {
  const transport = useTransport();
  useEffect(() => {
    const root = layer.current.rootRef.current;
    if (!online || !root) return;
    let last: ArrangerPointer | null = null;
    let client: { x: number; y: number } | null = null;
    let frame = 0;
    const send = (pointer: ArrangerPointer | null) => {
      if (samePointer(pointer, last)) return;
      last = pointer;
      void transport.send(cmd("Collab", { type: "SetPointer", pointer })).catch(() => undefined);
    };
    const compute = (): ArrangerPointer | null => {
      const { rootRef, scrollRef, rows, masterRow } = layer.current;
      const rootEl = rootRef.current;
      const scroll = scrollRef.current;
      if (!client || !rootEl || !scroll) return null;
      const g = measure(rootEl, scroll, rows, masterRow);
      const r = rootEl.getBoundingClientRect();
      const x = client.x - r.left;
      const y = client.y - r.top;
      const bottom = g.masterBox?.y1 ?? g.scrollBox.y1;
      // Only over the ruler and the tracks (not the toolbar or the scrollbar).
      if (y < g.rulerTop || y >= bottom || x >= g.scrollBox.x1) return null;
      if (y < g.scrollBox.y0) return { ...screenToSong(x, y, [], g.lanes), track: null, y: 0 };
      const inMaster = g.masterBox && y >= g.masterBox.y0;
      const band = g.rows.filter((row) => (row.track === g.masterTrack) === !!inMaster);
      // Below the last track: a fraction of the free space (at least the drop area tall).
      return screenToSong(x, y, band, g.lanes, inMaster ? undefined : g.free, DROP_AREA_HEIGHT);
    };
    const flush = () => {
      frame = 0;
      if (client) send(compute());
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(flush);
    };
    const onMove = (e: PointerEvent) => {
      client = { x: e.clientX, y: e.clientY };
      schedule();
    };
    const clear = () => {
      client = null;
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      send(null);
    };
    const onVisibility = () => {
      if (document.hidden) clear();
    };
    // Scrolling or zooming under a still pointer moves it in the song.
    const offView = arrangementView.subscribe(() => client && schedule());
    const scroll = layer.current.scrollRef.current;
    root.addEventListener("pointermove", onMove);
    root.addEventListener("pointerleave", clear);
    scroll?.addEventListener("scroll", schedule);
    window.addEventListener("blur", clear);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      root.removeEventListener("pointermove", onMove);
      root.removeEventListener("pointerleave", clear);
      scroll?.removeEventListener("scroll", schedule);
      window.removeEventListener("blur", clear);
      document.removeEventListener("visibilitychange", onVisibility);
      offView();
      clear();
    };
  }, [layer, online, transport]);
}

/** Keep `useLocalPresence.viewport` up to date (published with the presence, ≤ 10 Hz). */
function usePublishViewport(layer: LayerRef, rows: ReadonlyArray<Row>) {
  useEffect(() => {
    const scroll = layer.current.scrollRef.current;
    if (!scroll) return;
    let frame = 0;
    let lastKey = "";
    const update = () => {
      frame = 0;
      const view = arrangementView.getState();
      if (view.widthPx <= 0) return;
      const v = viewportOf(contentRows(rows), scroll.scrollTop, view.visibleRange());
      const key = JSON.stringify(v);
      if (key === lastKey) return;
      lastKey = key;
      useLocalPresence.setState({ viewport: v });
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(update);
    };
    update();
    const off = arrangementView.subscribe(schedule);
    scroll.addEventListener("scroll", schedule);
    return () => {
      off();
      scroll.removeEventListener("scroll", schedule);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [layer, rows]);
  useEffect(() => () => useLocalPresence.setState({ viewport: null }), []);
}

/**
 * Follow mode: apply the followed peer's viewport on each of its presence updates; any local
 * scroll or zoom (wheel, middle-button pan, scrollbar, zoom controls) or Escape stops it.
 */
function useFollow(layer: LayerRef, online: boolean) {
  useEffect(() => {
    const root = layer.current.rootRef.current;
    const scroll = layer.current.scrollRef.current;
    if (!online || !root || !scroll) return;
    let applied: { scrollTop: number; pxPerBeat: number } | null = null;
    let applying = false;
    const apply = () => {
      const following = useLocalPresence.getState().following;
      if (!following) {
        applied = null;
        return;
      }
      const leader = useCollabStore.getState().peers.find((p) => p.site === following);
      if (!leader) {
        setFollowing(null);
        return;
      }
      const v = leader.state.viewport;
      if (!v || !(v.end > v.start)) return;
      applying = true;
      try {
        arrangementView.getState().zoomToRange({ start: v.start, end: v.end });
        const top = scrollTopFor(v, contentRows(layer.current.rows), parentOf);
        if (top !== null) scroll.scrollTop = top;
        applied = { scrollTop: scroll.scrollTop, pxPerBeat: arrangementView.getState().pxPerBeat };
      } finally {
        applying = false;
      }
    };
    const stop = () => {
      if (useLocalPresence.getState().following) setFollowing(null);
    };
    const onScroll = () => {
      if (!applying && applied && Math.abs(scroll.scrollTop - applied.scrollTop) > 1) stop();
    };
    const onPointerDown = (e: PointerEvent) => {
      if (e.button === 1) stop();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") stop();
    };
    const offView = arrangementView.subscribe((s) => {
      // The playhead may scroll the view while playing; only a zoom counts as local here.
      if (!applying && applied && Math.abs(s.pxPerBeat - applied.pxPerBeat) > 1e-9) stop();
    });
    const offPeers = useCollabStore.subscribe((s, prev) => {
      if (s.peers !== prev.peers) apply();
    });
    const offLocal = useLocalPresence.subscribe((s, prev) => {
      if (s.following !== prev.following) apply();
    });
    root.addEventListener("wheel", stop, { capture: true, passive: true });
    root.addEventListener("pointerdown", onPointerDown, { capture: true });
    scroll.addEventListener("scroll", onScroll);
    window.addEventListener("keydown", onKey);
    apply();
    return () => {
      offView();
      offPeers();
      offLocal();
      root.removeEventListener("wheel", stop, { capture: true });
      root.removeEventListener("pointerdown", onPointerDown, { capture: true });
      scroll.removeEventListener("scroll", onScroll);
      window.removeEventListener("keydown", onKey);
    };
  }, [layer, online]);
}

/** The peers' pointers, drawn in one layer (pointer-events: none) over the arrangement. */
function PeerPointers({ layer }: { layer: LayerRef }) {
  const sites = usePointerStore((s) => s.sites);
  const peers = useCollabStore((s) => s.peers);
  const els = useRef(new Map<SiteId, HTMLDivElement>());

  useEffect(() => {
    if (sites.length === 0) return;
    let frame = 0;
    const draw = () => {
      frame = requestAnimationFrame(draw);
      const { rootRef, scrollRef, rows, masterRow } = layer.current;
      const root = rootRef.current;
      const scroll = scrollRef.current;
      if (!root || !scroll) return;
      const g = measure(root, scroll, rows, masterRow);
      const now = performance.now();
      const { trails } = usePointerStore.getState();
      for (const [site, el] of els.current) {
        const p = trails.get(site)?.at(now) ?? null;
        // In a piano roll: drawn there (see EditorPointers), not over the arranger.
        const at = p && !p.editor ? songToScreen(p, g.rows, g.lanes, parentOf, g.rulerY, g.free, DROP_AREA_HEIGHT) : null;
        if (!p || !at) {
          el.dataset.hidden = "true";
          continue;
        }
        const box =
          p.track === null
            ? p.y > 0
              ? g.scrollBox
              : { ...g.scrollBox, y0: g.rulerTop, y1: g.scrollBox.y1 }
            : p.track === g.masterTrack && g.masterBox
              ? g.masterBox
              : g.scrollBox;
        const c = clampToBox(at.x, at.y, box);
        el.dataset.hidden = "false";
        el.dataset.edge = c.edge ?? "";
        el.dataset.folded = String(at.folded);
        el.dataset.beats = p.beats.toFixed(2);
        el.dataset.track = p.track ?? "";
        el.style.transform = `translate(${c.x}px, ${c.y}px)`;
      }
    };
    draw();
    return () => cancelAnimationFrame(frame);
  }, [sites, layer]);

  return (
    <div className="eth-collab-pointers" aria-hidden data-testid="peer-pointers">
      {sites.map((site) => {
        const peer = peers.find((p) => p.site === site);
        const name = peer?.name || "Anonymous";
        const activity = activityLabel(peer?.state.activity);
        return (
          <div
            key={site}
            ref={(el) => {
              if (el) els.current.set(site, el);
              else els.current.delete(site);
            }}
            className="eth-collab-pointer"
            data-testid="peer-pointer"
            data-peer={name}
            data-hidden="true"
            style={{ "--eth-collab-peer": peer ? peerColor(peer.color) : "var(--eth-color-text-dim)" } as CSSProperties}
          >
            <svg className="eth-collab-pointer__arrow" viewBox="0 0 12 16" aria-hidden>
              <path d="M0 0 L0 13 L3.5 9.8 L6 15.5 L8.2 14.5 L5.8 9 L10.5 9 Z" />
            </svg>
            <span className="eth-collab-pointer__label">{activity ? `${name} · ${activity}` : name}</span>
          </div>
        );
      })}
    </div>
  );
}
