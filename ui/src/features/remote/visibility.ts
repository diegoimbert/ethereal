// #717: `ether-server` isn't in any release, so Settings > Advanced shows the engine server
// form only to those who turned it on: the palette's "Engine server…", `?engine-server` in
// the web app's address, or an address already used (`URL_KEY`). Kept on this computer.
import { create } from "zustand";
import { URL_KEY } from "./engineServer";

export const ENGINE_SERVER_SHOWN_KEY = "eth.engine-server";
/** The web app's address parameter that turns it on (`/?engine-server`). */
export const ENGINE_SERVER_PARAM = "engine-server";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/** Turned on before (or by this page's address), or an engine server was used before. */
export function engineServerWanted(): boolean {
  try {
    if (new URLSearchParams(window.location.search).has(ENGINE_SERVER_PARAM)) {
      localStorage.setItem(ENGINE_SERVER_SHOWN_KEY, "on");
      return true;
    }
  } catch {
    /* no storage: this page only */
  }
  return read(ENGINE_SERVER_SHOWN_KEY) === "on" || !!read(URL_KEY);
}

interface State {
  shown: boolean;
  show(): void;
}

export const useEngineServerShown = create<State>()((set) => ({
  shown: engineServerWanted(),
  show: () => {
    try {
      localStorage.setItem(ENGINE_SERVER_SHOWN_KEY, "on");
    } catch {
      /* this session only */
    }
    set({ shown: true });
  },
}));

/** Tests: read again from storage and the address. */
export function reloadEngineServerShown(): void {
  useEngineServerShown.setState({ shown: engineServerWanted() });
}
