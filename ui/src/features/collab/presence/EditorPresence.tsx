// Presence inside an editor (the piano roll): this user's editing clip and pointer are
// published, peers' pointers on the same clip are drawn over the grid, and `EditingPeers`
// shows who else has the clip open. The editor supplies the coordinate mapping, so this
// module knows nothing about keys or rows.
import { useEffect, useLayoutEffect, useRef, type CSSProperties, type RefObject } from "react";
import type { ArrangerPointer, ClipId, SiteId } from "@/generated";
import { cmd, useTransport } from "@/transport";
import { peerColor, useCollabStore } from "../store";
import { useClipEditors } from "./editors";
import { setEditingClip, useLocalPresence } from "./local";
import { usePointerStore } from "./pointers";

/** Grid px (relative to the grid element's top-left) ⇄ the clip's content beats and pitch. */
export interface EditorCursorMapping {
  /** Content position under a grid point (`pitch`: the key plus how far up it, see `EditorPointer`). */
  fromScreen(x: number, y: number): { beats: number; pitch: number };
  toScreen(beats: number, pitch: number): { x: number; y: number };
}

export interface EditorPresenceProps {
  clip: ClipId;
  /** The grid element (pointer events and the overlay's coordinate space). */
  gridRef: RefObject<HTMLElement | null>;
  /** Read on every event and frame (keep it current with the editor's zoom and scroll). */
  mapping: RefObject<EditorCursorMapping>;
}

const EPS = 1e-4;
const sameEditorPointer = (a: ArrangerPointer | null, b: ArrangerPointer | null) =>
  a === b ||
  (!!a?.editor &&
    !!b?.editor &&
    a.editor.clip === b.editor.clip &&
    Math.abs(a.editor.beats - b.editor.beats) < EPS &&
    Math.abs(a.editor.pitch - b.editor.pitch) < EPS);

/** Render inside the grid (it fills it). Reports the edited clip even when offline. */
export function EditorPresence({ clip, gridRef, mapping }: EditorPresenceProps) {
  const online = useCollabStore((s) => s.status.type === "Online");
  useEffect(() => {
    setEditingClip(clip);
    return () => {
      if (useLocalPresence.getState().editingClip === clip) setEditingClip(null);
    };
  }, [clip]);
  usePublishEditorPointer(clip, gridRef, mapping, online);
  return online ? <EditorPointers clip={clip} mapping={mapping} /> : null;
}

function usePublishEditorPointer(
  clip: ClipId,
  gridRef: RefObject<HTMLElement | null>,
  mapping: RefObject<EditorCursorMapping>,
  online: boolean,
) {
  const transport = useTransport();
  useEffect(() => {
    const grid = gridRef.current;
    if (!online || !grid) return;
    let last: ArrangerPointer | null = null;
    let client: { x: number; y: number } | null = null;
    let frame = 0;
    const send = (pointer: ArrangerPointer | null) => {
      if (sameEditorPointer(pointer, last)) return;
      last = pointer;
      void transport.send(cmd("Collab", { type: "SetPointer", pointer })).catch(() => undefined);
    };
    const flush = () => {
      frame = 0;
      if (!client) return;
      const box = grid.getBoundingClientRect();
      const { beats, pitch } = mapping.current.fromScreen(client.x - box.left, client.y - box.top);
      send({ beats: 0, track: null, y: 0, editor: { clip, beats: Math.max(0, beats), pitch } });
    };
    const schedule = () => {
      if (client && !frame) frame = requestAnimationFrame(flush);
    };
    const onMove = (e: PointerEvent) => {
      client = { x: e.clientX, y: e.clientY };
      schedule();
    };
    const onLeave = () => {
      client = null;
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      send(null);
    };
    grid.addEventListener("pointermove", onMove);
    grid.addEventListener("pointerleave", onLeave);
    // Scrolling or zooming moves the content under a still pointer.
    window.addEventListener("scroll", schedule, { capture: true, passive: true });
    window.addEventListener("wheel", schedule, { capture: true, passive: true });
    return () => {
      grid.removeEventListener("pointermove", onMove);
      grid.removeEventListener("pointerleave", onLeave);
      window.removeEventListener("scroll", schedule, { capture: true });
      window.removeEventListener("wheel", schedule, { capture: true });
      if (frame) cancelAnimationFrame(frame);
      if (last) send(null);
    };
  }, [clip, gridRef, mapping, online, transport]);
}

/** Peers' pointers on this clip, drawn at display rate (pointer-events: none). */
function EditorPointers({ clip, mapping }: { clip: ClipId; mapping: RefObject<EditorCursorMapping> }) {
  const sites = usePointerStore((s) => s.sites);
  const peers = useCollabStore((s) => s.peers);
  const els = useRef(new Map<SiteId, HTMLDivElement>());
  const clipRef = useRef(clip);
  useLayoutEffect(() => {
    clipRef.current = clip;
  });

  useEffect(() => {
    if (sites.length === 0) return;
    let frame = 0;
    const draw = () => {
      frame = requestAnimationFrame(draw);
      const now = performance.now();
      const { trails } = usePointerStore.getState();
      for (const [site, el] of els.current) {
        const e = trails.get(site)?.at(now)?.editor;
        if (!e || e.clip !== clipRef.current) {
          el.dataset.hidden = "true";
          continue;
        }
        const { x, y } = mapping.current.toScreen(e.beats, e.pitch);
        el.dataset.hidden = "false";
        el.style.transform = `translate(${x}px, ${y}px)`;
      }
    };
    draw();
    return () => cancelAnimationFrame(frame);
  }, [sites, mapping]);

  return (
    <div className="eth-collab-pointers" aria-hidden data-testid="editor-peer-pointers">
      {sites.map((site) => {
        const peer = peers.find((p) => p.site === site);
        const name = peer?.name || "Anonymous";
        return (
          <div
            key={site}
            ref={(el) => {
              if (el) els.current.set(site, el);
              else els.current.delete(site);
            }}
            className="eth-collab-pointer"
            data-testid="editor-peer-pointer"
            data-peer={name}
            data-hidden="true"
            style={{ "--eth-collab-peer": peer ? peerColor(peer.color) : "var(--eth-color-text-dim)" } as CSSProperties}
          >
            <svg className="eth-collab-pointer__arrow" viewBox="0 0 12 16" aria-hidden>
              <path d="M0 0 L0 13 L3.5 9.8 L6 15.5 L8.2 14.5 L5.8 9 L10.5 9 Z" />
            </svg>
            <span className="eth-collab-pointer__label">{name}</span>
          </div>
        );
      })}
    </div>
  );
}

/** "Ada is editing" chips for the editor's header. */
export function EditingPeers({ clip }: { clip: ClipId }) {
  const editors = useClipEditors(clip);
  if (editors.length === 0) return null;
  const names = editors.map((e) => e.name);
  const label = `${names.join(", ")} ${names.length === 1 ? "is" : "are"} editing this clip`;
  return (
    <span className="eth-collab-editing" role="status" aria-label={label} title={label}>
      {editors.map((e) => (
        <span key={e.site} className="eth-collab-editing__chip" style={{ "--eth-collab-peer": e.color } as CSSProperties}>
          {e.name}
        </span>
      ))}
      <span className="eth-collab-editing__text">{names.length === 1 ? "is editing" : "are editing"}</span>
    </span>
  );
}
