// Hosting state ("listen on <peer>", host side; docs/COLLAB.md §9.3): this site's hosting
// preferences, its current listeners (`ListenStatus.listeners`), the ICE servers, and the
// web sender's link states.

import { create } from "zustand";
import type { Event, IceServer, ListenerLink } from "@/generated";
import type { LinkState } from "./sender";

/** Remembered hosting preferences. */
const PREFS_KEY = "eth-collab-hosting";

interface Prefs {
  allow: boolean;
  remoteTransport: boolean;
}

function loadPrefs(): Prefs {
  try {
    const v = JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") as Partial<Prefs>;
    return { allow: v.allow ?? true, remoteTransport: v.remoteTransport ?? true };
  } catch {
    return { allow: true, remoteTransport: true };
  }
}

function savePrefs(p: Prefs) {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    // storage unavailable: defaults next time
  }
}

export interface HostState {
  /** Let session members listen to this computer (`SetHosting.allow`). */
  allow: boolean;
  /** Listeners may play/stop/locate/loop (`SetHosting.remote_transport`). */
  remoteTransport: boolean;
  /** This UI streams the engine itself (web build; `SetHosting.ui_sender`). */
  uiSender: boolean;
  /** Who listens to this site (`ListenStatus.listeners`). */
  listeners: ListenerLink[];
  /** Web sender link states, by `linkKey(site, stream)`. */
  links: Record<string, LinkState>;
  iceServers: IceServer[];
  setAllow(allow: boolean): void;
  setRemoteTransport(remoteTransport: boolean): void;
  setUiSender(uiSender: boolean): void;
  setLinks(links: ReadonlyMap<string, LinkState>): void;
  /** Apply one engine event (only `Collab` `ListenStatus` / `IceServers`). */
  onEvent(event: Event): void;
  /** Leave the session: no listeners, no links. */
  clearSession(): void;
}

export const useHostStore = create<HostState>()((set, get) => ({
  ...loadPrefs(),
  uiSender: false,
  listeners: [],
  links: {},
  iceServers: [],
  setAllow: (allow) => {
    set({ allow });
    savePrefs({ allow, remoteTransport: get().remoteTransport });
  },
  setRemoteTransport: (remoteTransport) => {
    set({ remoteTransport });
    savePrefs({ allow: get().allow, remoteTransport });
  },
  setUiSender: (uiSender) => set({ uiSender }),
  setLinks: (links) => set({ links: Object.fromEntries(links) }),
  onEvent: (event) => {
    if (event.type !== "Collab") return;
    const e = event.event;
    if (e.type === "ListenStatus") set({ listeners: e.status.listeners });
    else if (e.type === "IceServers") set({ iceServers: e.servers });
  },
  clearSession: () => set({ listeners: [], links: {} }),
}));
