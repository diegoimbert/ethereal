// Collaboration UI state, mirrored from `Event::Collab` (docs/COLLAB.md): the session status
// and the other participants' presence.
import { create } from "zustand";
import type { CollabStatus, Color, Event, Presence } from "@/generated";

export interface CollabState {
  status: CollabStatus;
  /** The other participants (never this site). */
  peers: Presence[];
  /** Apply one engine event (ignores non-collab events). */
  onEvent(event: Event): void;
  reset(): void;
}

const INITIAL = { status: { type: "Offline" } as CollabStatus, peers: [] as Presence[] };

export const useCollabStore = create<CollabState>()((set) => ({
  ...INITIAL,
  onEvent: (event) => {
    if (event.type !== "Collab") return;
    const e = event.event;
    if (e.type === "Session") set({ status: e.status });
    else if (e.type === "Presence") set({ peers: e.peers });
    // base-53 events (Pointer, Signal, ListenStatus, StreamClock, IceServers) are handled by
    // the presence-v2 / stream-listen / stream-host nodes.
  },
  reset: () => set({ ...INITIAL }),
}));

/** A presence color (`0xRRGGBB`, data like track colors) as CSS. */
export function peerColor(color: Color): string {
  return `#${(color & 0xffffff).toString(16).padStart(6, "0")}`;
}

/** Initials for an avatar chip ("Ada Lovelace" → "AL", "" → "?"). */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  return words
    .slice(0, 2)
    .map((w) => [...w][0]!.toUpperCase())
    .join("");
}

/**
 * CSS rules outlining what each peer has selected, in its color: track rows / mixer strips
 * (`[data-track]`) and clips (`[data-clip-id]`). Ids are ULIDs (safe in selectors), but
 * anything else is dropped.
 */
export function highlightCss(peers: Presence[]): string {
  const safe = (id: string) => /^[0-9A-Za-z]+$/.test(id);
  const rules: string[] = [];
  for (const p of peers) {
    const color = peerColor(p.color);
    const tracks = p.state.selected_tracks.filter(safe);
    const clips = p.state.selected_clips.filter(safe);
    if (tracks.length) {
      const sel = tracks.map((id) => `.eth-arr-row[data-track="${id}"], .eth-strip[data-track="${id}"]`).join(", ");
      rules.push(`${sel} { box-shadow: inset 0 0 0 var(--eth-collab-outline-width) ${color}; }`);
    }
    if (clips.length) {
      const sel = clips.map((id) => `[data-clip-id="${id}"]`).join(", ");
      rules.push(
        `${sel} { outline: var(--eth-collab-outline-width) solid ${color}; outline-offset: calc(-1 * var(--eth-collab-outline-width)); }`,
      );
    }
  }
  return rules.join("\n");
}
