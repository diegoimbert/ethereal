// Peers' playheads inside an editor (the piano roll), like the arranger's (collab-social,
// docs/COLLAB.md §12.3): one dashed line per peer whose transport is known, in its colour,
// extrapolated per frame and mapped from song time onto the clip's content axis. A peer
// playing outside the clip (or in a part of the song the clip doesn't cover) has no line.
// Mounted by `EditorPresence`, so it shares its "online and not hidden" gate.
import { useEffect, useRef, type RefObject } from "react";
import type { Beats, ClipId, SiteId } from "@/generated";
import { songToContent } from "@/features/piano-roll/clipTime";
import { useProjectStore } from "@/state";
import { useTempoMap } from "@/timeline";
import { activeHost } from "../listen/store";
import { useListenStore } from "../listen";
import { extrapolate, SampleTracker } from "../social/playheads/extrapolate";
import { initials, useCollabStore } from "../store";
import { avatarStyle } from "./avatar";
import "./presence.css";
import type { EditorCursorMapping } from "./EditorPresence";

/**
 * Content x of a peer at song position `song` in `clip`, or `null` when the clip doesn't
 * play there. Pure (the per-frame part of `EditorPlayheads`).
 */
export function editorPlayheadX(
  clip: Parameters<typeof songToContent>[0] | undefined,
  song: Beats,
  mapping: Pick<EditorCursorMapping, "toScreen">,
): number | null {
  if (!clip) return null;
  const content = songToContent(clip, song);
  return content === null ? null : mapping.toScreen(content, 0).x;
}

/** The nearest ancestor that scrolls vertically (the editor's body), if any. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    if (/(auto|scroll)/.test(getComputedStyle(p).overflowY)) return p;
  }
  return null;
}

export function EditorPlayheads({ clip, mapping }: { clip: ClipId; mapping: RefObject<EditorCursorMapping> }) {
  const peers = useCollabStore((s) => s.peers);
  const listeningTo = useListenStore((s) => activeHost(s.listening));
  const tempo = useTempoMap();
  const tracker = useRef(new SampleTracker());
  const els = useRef(new Map<SiteId, HTMLDivElement>());
  const root = useRef<HTMLDivElement>(null);
  // The host we listen to already drives our own playhead (stream-listen): not twice.
  const withTransport = peers.filter((p) => p.state.transport && p.site !== listeningTo);
  const sites = withTransport.map((p) => p.site).join(",");

  useEffect(() => {
    tracker.current.update(peers, performance.now());
  }, [peers]);

  useEffect(() => {
    if (!sites) return;
    let frame = 0;
    const scroller = root.current ? scrollParent(root.current) : null;
    const draw = () => {
      frame = requestAnimationFrame(draw);
      const now = performance.now();
      const c = useProjectStore.getState().project?.clips[clip];
      // The initials caps ride the top of the visible part of the grid (it scrolls under the
      // editor's keys; `position: sticky` can't, the grid clips its overflow).
      const overlay = root.current;
      if (overlay && scroller) {
        const top = Math.max(0, scroller.getBoundingClientRect().top - overlay.getBoundingClientRect().top);
        overlay.style.setProperty("--eth-collab-cap-top", `${top}px`);
      }
      for (const [site, el] of els.current) {
        const sample = tracker.current.get(site);
        const beats = sample ? extrapolate(sample, now, tempo) : null;
        const x = beats === null ? null : editorPlayheadX(c, beats, mapping.current);
        if (sample && beats !== null) {
          el.dataset.playing = String(sample.transport.playing);
          el.dataset.beats = beats.toFixed(3);
        }
        if (x === null) {
          el.dataset.hidden = "true";
          continue;
        }
        el.dataset.hidden = "false";
        el.style.transform = `translateX(${x}px)`;
      }
    };
    draw();
    return () => cancelAnimationFrame(frame);
  }, [sites, clip, mapping, tempo]);

  if (withTransport.length === 0) return null;
  return (
    <div ref={root} className="eth-collab-editor-playheads" aria-hidden data-testid="editor-peer-playheads">
      {withTransport.map((p) => (
        <div
          key={p.site}
          ref={(el) => {
            if (el) els.current.set(p.site, el);
            else els.current.delete(p.site);
          }}
          className="eth-collab-editor-playhead"
          data-testid="editor-peer-playhead"
          data-peer={p.name || "Anonymous"}
          data-hidden="true"
          style={avatarStyle(p.color)}
        >
          <span className="eth-collab-editor-playhead__cap">{initials(p.name)}</span>
        </div>
      ))}
    </div>
  );
}
