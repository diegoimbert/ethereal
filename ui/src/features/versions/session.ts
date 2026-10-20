// Did the previous session close cleanly? Read by "Reopen last project on
// launch" (never after a crash) and the crash dialog.
//
// - Natively (and on a remote engine), the engine knows: opening a project writes its
//   session marker before any device or plugin is loaded, a clean quit removes it, so a
//   marker left by another session is a crash (`Version::SessionStatus`).
// - In the browser the engine Worker dies with the tab and never closes cleanly, so its
//   markers always survive. The page records it instead (localStorage, written
//   synchronously): "open" once connected, "closed" when the page is unloaded with a
//   healthy engine. A tab that crashed or was killed never reaches "closed".
import type { ProjectId } from "@/generated";
import { cmd, WasmTransport, type EngineTransport } from "@/transport";

/** localStorage: how the last browser session ended (`"open"` = it never closed cleanly). */
export const WEB_SESSION_KEY = "eth.session";
/** localStorage: id of the last project opened (shared with the project feature). */
export const LAST_PROJECT_KEY = "eth.project.last";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage unavailable: the next launch treats this session as clean.
  }
}

/**
 * How the previous browser session ended: read when the page loads, and again when this
 * session starts (the first `trackWebSession`: the tab that had the store before may have
 * handed it over since).
 */
let previousWebSession = read(WEB_SESSION_KEY);
let sessionStarted = false;

/** This tab handed the store to another one after saving: a clean close. */
export function markWebSessionClosed(): void {
  write(WEB_SESSION_KEY, "closed");
}

export interface PreviousSession {
  /** It closed cleanly. */
  clean: boolean;
  /** Projects it left open (to offer in the crash dialog). */
  unclean: ProjectId[];
}

/** How the session before this one ended. */
export async function previousSession(transport: EngineTransport): Promise<PreviousSession> {
  const reply = await transport.send(cmd("Version", { type: "SessionStatus" }));
  const markers = reply.type === "SessionStatus" ? reply.unclean : [];
  if (transport.kind !== "wasm") return { clean: markers.length === 0, unclean: markers };
  if (previousWebSession !== "open") return { clean: true, unclean: [] };
  // Every marker survives in the browser: only the project open at the crash counts.
  const last = read(LAST_PROJECT_KEY);
  return { clean: false, unclean: markers.filter((id) => id === last) };
}

/**
 * Browser only, before the engine is up: the session before this one never closed cleanly
 * (a crash, or a killed tab). Launch then waits for the crash dialog instead of showing the
 * picker early.
 */
export function webSessionLeftOpen(): boolean {
  return (sessionStarted ? previousWebSession : read(WEB_SESSION_KEY)) === "open";
}

/**
 * Browser only: mark this session open once connected, and closed when the page is
 * unloaded while the engine is healthy. Returns the cleanup.
 */
export function trackWebSession(transport: EngineTransport): () => void {
  if (!(transport instanceof WasmTransport)) return () => undefined;
  if (!sessionStarted) {
    sessionStarted = true;
    previousWebSession = read(WEB_SESSION_KEY);
  }
  write(WEB_SESSION_KEY, "open");
  const onHide = (e: PageTransitionEvent) => {
    // Kept in the back/forward cache: the session goes on.
    if (e.persisted || transport.failed) return;
    write(WEB_SESSION_KEY, "closed");
  };
  const onShow = (e: PageTransitionEvent) => {
    if (e.persisted) write(WEB_SESSION_KEY, "open");
  };
  window.addEventListener("pagehide", onHide);
  window.addEventListener("pageshow", onShow);
  return () => {
    window.removeEventListener("pagehide", onHide);
    window.removeEventListener("pageshow", onShow);
  };
}
