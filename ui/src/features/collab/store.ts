// Collaboration UI state, mirrored from `Event::Collab` (docs/COLLAB.md): the session status
// and the other participants' presence.
import { create } from "zustand";
import type { CollabStatus, Color, Event, Presence } from "@/generated";
import { usePointerStore } from "./presence/pointers";
import { onChatReceived } from "./social/chatStore";

export interface CollabState {
  status: CollabStatus;
  /** The other participants (never this site). */
  peers: Presence[];
  /**
   * "Hide users and notes" (docs/COLLAB.md §12.4): a local preference (localStorage, never
   * sent). Every presence and notes renderer honours it via `useHideOthers()`.
   */
  hideOthers: boolean;
  setHideOthers(hide: boolean): void;
  /** Apply one engine event (ignores non-collab events). */
  onEvent(event: Event): void;
  reset(): void;
}

const INITIAL = { status: { type: "Offline" } as CollabStatus, peers: [] as Presence[] };

/** Stored next to the dialog's remembered join fields. */
const HIDE_KEY = "eth-collab-hide-others";

function loadHide(): boolean {
  try {
    return localStorage.getItem(HIDE_KEY) === "1";
  } catch {
    return false;
  }
}

export const useCollabStore = create<CollabState>()((set) => ({
  ...INITIAL,
  hideOthers: loadHide(),
  setHideOthers: (hide) => {
    set({ hideOthers: hide });
    try {
      localStorage.setItem(HIDE_KEY, hide ? "1" : "0");
    } catch {
      // storage unavailable: this session only
    }
  },
  onEvent: (event) => {
    if (event.type !== "Collab") return;
    const e = event.event;
    if (e.type === "Session") {
      set({ status: e.status });
      if (e.status.type !== "Online") usePointerStore.getState().clear();
    } else if (e.type === "Presence") set({ peers: e.peers });
    // presence-v2: peers' live pointers (their own store: they arrive at up to 30 Hz).
    else if (e.type === "Pointer") usePointerStore.getState().onPointer(e.site, e.pointer);
    // collab-social: peers' live chat messages (toasted while the chat is closed).
    else if (e.type === "ChatReceived") onChatReceived(e.ids);
    // base-53 events (Signal, ListenStatus, StreamClock, IceServers) are handled by the
    // stream-listen / stream-host nodes.
  },
  reset: () => {
    set({ ...INITIAL });
    usePointerStore.getState().clear();
  },
}));

/** "Hide users and notes" is on: draw no peers' pointers, playheads, outlines or notes. */
export function useHideOthers(): boolean {
  return useCollabStore((s) => s.hideOthers);
}

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
