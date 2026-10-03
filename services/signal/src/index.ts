/**
 * Ethereal signaling service (docs/SHARING.md §3): introduces sharing peers and forwards
 * their SDP/ICE. Never sees a project, a link key or a document op.
 *
 *   GET /v1/health                       → 200 {"ok":true,"protocol":1}
 *   GET /v1/rooms/<room>/host  (upgrade) → the host's socket (HostHello first)
 *   GET /v1/rooms/<room>/join  (upgrade) → a joiner's socket (JoinHello first)
 *
 * Also mounted under `/signal/...` (the Pages Function proxy on the web app's origin). The
 * room object repeats the origin and upgrade checks, so every front door is equally strict.
 */

import { upgradeError } from "./do.ts";
import { originAllowed, route, SIGNAL_PROTOCOL_VERSION } from "./protocol.ts";

export { IpBudget, SignalRoom } from "./do.ts";

function cors(origin: string | null, env: Env): HeadersInit {
  return origin && originAllowed(origin, env.ALLOWED_ORIGINS)
    ? { "Access-Control-Allow-Origin": origin, Vary: "Origin", "Cross-Origin-Resource-Policy": "cross-origin" }
    : {};
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const origin = request.headers.get("Origin");
    const r = route(url.pathname);
    if (!r) return new Response("not found", { status: 404 });
    if (!originAllowed(origin, env.ALLOWED_ORIGINS)) return new Response("origin not allowed", { status: 403 });
    if (r.kind === "health") {
      return Response.json({ ok: true, protocol: SIGNAL_PROTOCOL_VERSION }, { headers: cors(origin, env) });
    }
    const err = upgradeError(request, env);
    if (err) return err;
    // Per-IP budgets (claims, bad doors) are enforced in the room, where the outcome is known.
    const stub = env.ROOMS.get(env.ROOMS.idFromName(r.room));
    return stub.fetch(request);
  },
};
