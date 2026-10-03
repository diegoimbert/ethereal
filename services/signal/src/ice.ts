/**
 * The ICE servers the service advertises in `HostWelcome`/`JoinWelcome` (docs/SHARING.md §2.4,
 * §3.4): the `STUN_URLS` list, plus short-lived TURN credentials when the operator configured
 * Cloudflare Realtime TURN (`TURN_KEY_ID` + `TURN_KEY_API_TOKEN` secrets).
 */

import type { IceServer } from "../../../ui/src/generated/index.ts";

export type { IceServer };

/** How long minted TURN credentials stay valid, and how long one answer is reused. */
export const TURN_TTL_S = 24 * 3600;
export const TURN_REUSE_MS = 3600_000;

const TURN_API = "https://rtc.live.cloudflare.com/v1/turn/keys";

/** `STUN_URLS` (comma-separated `stun:` URLs) as one `IceServer`, or none. */
export function stunServers(urls: string | undefined): IceServer[] {
  const list = (urls ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter((s) => /^stuns?:/.test(s));
  return list.length ? [{ urls: list, username: null, credential: null }] : [];
}

/** One entry of Cloudflare's answer (`urls` may be a string or an array). */
function toIceServer(v: unknown): IceServer | null {
  if (typeof v !== "object" || v === null) return null;
  const o = v as Record<string, unknown>;
  const urls = (typeof o.urls === "string" ? [o.urls] : Array.isArray(o.urls) ? o.urls : [])
    .filter((u): u is string => typeof u === "string" && /^(stuns?|turns?):/.test(u))
    // Browsers block port 53; Cloudflare's own docs say to drop those URLs.
    .filter((u) => !/:53(\?|$)/.test(u));
  if (!urls.length) return null;
  const username = typeof o.username === "string" ? o.username : null;
  const credential = typeof o.credential === "string" ? o.credential : null;
  return { urls, username, credential };
}

type Fetch = (input: string, init: RequestInit) => Promise<Response>;

/**
 * Mint TURN credentials (`POST /credentials/generate-ice-servers`). Only the TURN entries
 * are kept (STUN comes from `STUN_URLS`). Any failure: `[]` (STUN only), never an error to
 * the client.
 */
export async function mintTurn(keyId: string, token: string, fetchFn: Fetch): Promise<IceServer[]> {
  try {
    const res = await fetchFn(`${TURN_API}/${encodeURIComponent(keyId)}/credentials/generate-ice-servers`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ ttl: TURN_TTL_S }),
    });
    if (!res.ok) return [];
    const body = (await res.json()) as { iceServers?: unknown };
    const raw = Array.isArray(body.iceServers) ? body.iceServers : [body.iceServers];
    return raw
      .map(toIceServer)
      .filter((s): s is IceServer => s !== null && s.username !== null && s.urls.some((u) => u.startsWith("turn")));
  } catch {
    return [];
  }
}

/** STUN plus cached TURN; what a room hands to every welcome. */
export class IceProvider {
  private turn: { at: number; servers: IceServer[] } | null = null;

  constructor(
    private readonly env: { STUN_URLS?: string; TURN_KEY_ID?: string; TURN_KEY_API_TOKEN?: string },
    private readonly fetchFn: Fetch,
    private readonly now: () => number,
  ) {}

  async servers(): Promise<IceServer[]> {
    const stun = stunServers(this.env.STUN_URLS);
    const { TURN_KEY_ID: id, TURN_KEY_API_TOKEN: token } = this.env;
    if (!id || !token) return stun;
    if (!this.turn || this.now() - this.turn.at > TURN_REUSE_MS) {
      const servers = await mintTurn(id, token, this.fetchFn);
      // A failed mint is retried on the next welcome, not cached for an hour.
      this.turn = servers.length ? { at: this.now(), servers } : null;
      return [...stun, ...servers];
    }
    return [...stun, ...this.turn.servers];
  }
}
