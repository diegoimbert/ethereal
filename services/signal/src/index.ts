/**
 * Ethereal signaling service (docs/SHARING.md §3): introduces sharing peers and forwards
 * their SDP/ICE. Never sees a project, a link key or a document op.
 *
 *   GET /v1/health                       → 200 {"ok":true,"protocol":1}
 *   GET /v1/rooms/<room>/host  (upgrade) → the host's socket (HostHello first)
 *   GET /v1/rooms/<room>/join  (upgrade) → a joiner's socket (JoinHello first)
 *
 * Also mounted under `/signal/...` (the Pages Function proxy on the web app's origin).
 */

import { originAllowed, route, SIGNAL_PROTOCOL_VERSION } from "./protocol";

export { SignalRoom } from "./room";

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
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("expected a WebSocket upgrade", { status: 426 });
    }
    // signal-service: per-IP rate limits (claims, bad doors) before reaching the room.
    const stub = env.ROOMS.get(env.ROOMS.idFromName(r.room));
    return stub.fetch(request);
  },
};
