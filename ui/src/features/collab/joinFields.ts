// Collaboration dialog helpers (base-114): join-field validation and the remembered token.
import type { EngineTransport } from "@/transport";

/** Longest session name the relay accepts (ether-collab wire.rs `valid_session_name`). */
export const SESSION_MAX = 64;

/** Why `session` can't be joined (same rule as the relay: 1-64 of `[A-Za-z0-9._-]`), or `null`. */
export function sessionNameError(session: string): string | null {
  if (!session) return "Enter a session name.";
  if (session.length > SESSION_MAX) return `Use at most ${SESSION_MAX} characters.`;
  if (!/^[A-Za-z0-9._-]+$/.test(session)) return "Use only letters, digits, “.”, “_” and “-” (no spaces).";
  return null;
}

/** Why `server` is not a relay address, or `null`. */
export function relayError(server: string): string | null {
  return /^wss?:\/\/\S+$/.test(server) ? null : "Enter the relay address, such as ws://studio.local:9003.";
}

/** The desktop transport's token store (`TauriTransport`: the app data dir). */
export interface TokenHost {
  loadCollabToken(): Promise<string | null>;
  saveCollabToken(token: string | null): Promise<void>;
}

function isTokenHost(t: EngineTransport | null | undefined): t is EngineTransport & TokenHost {
  const h = t as Partial<TokenHost> | null | undefined;
  return typeof h?.loadCollabToken === "function" && typeof h.saveCollabToken === "function";
}

/** localStorage key of the web build's remembered token. */
export const WEB_TOKEN_KEY = "eth-collab-token";

export interface TokenStorage {
  /** Where the token is kept, for the dialog's note. */
  where: "app" | "browser";
  load(): Promise<string | null>;
  /** `null` forgets it. */
  save(token: string | null): Promise<void>;
}

/**
 * Where the relay token is remembered: the desktop app's data folder, else this browser's
 * localStorage. Failures are swallowed (and the token never logged): remembering is a
 * convenience.
 */
export function tokenStorage(transport: EngineTransport | null | undefined): TokenStorage {
  if (isTokenHost(transport)) {
    return {
      where: "app",
      load: () => transport.loadCollabToken().catch(() => null),
      save: (token) => transport.saveCollabToken(token).catch(() => undefined),
    };
  }
  return {
    where: "browser",
    load: () => {
      try {
        return Promise.resolve(localStorage.getItem(WEB_TOKEN_KEY));
      } catch {
        return Promise.resolve(null);
      }
    },
    save: (token) => {
      try {
        if (token) localStorage.setItem(WEB_TOKEN_KEY, token);
        else localStorage.removeItem(WEB_TOKEN_KEY);
      } catch {
        // storage unavailable: not remembered
      }
      return Promise.resolve();
    },
  };
}
