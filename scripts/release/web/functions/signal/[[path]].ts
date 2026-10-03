/**
 * Cloudflare Pages Function: mounts the signaling service (services/signal, docs/SHARING.md
 * §3.1) at `<web app origin>/signal/...`, so the app reaches it same-origin (no CORS, no CORP
 * friction under the app's COEP, no second domain).
 *
 * It forwards to the service's `SignalRoom` Durable Object through a binding set in the Pages
 * project (Settings > Bindings > Durable Object: name `ROOMS`, script `ethereal-signal`,
 * class `SignalRoom`). The room object itself checks the Origin and the upgrade, and applies
 * every limit, so this file stays a dumb router. Without the binding the routes answer 503.
 *
 * Self-contained on purpose (no imports): this folder ships in the web zip for self-hosters.
 */

interface PagesEnv {
  ROOMS?: {
    idFromName(name: string): unknown;
    get(id: unknown): { fetch(request: Request): Promise<Response> };
  };
}

/** `/signal/v1/rooms/<22 base64url chars>/(host|join)`, as in services/signal/src/protocol.ts. */
const ROOM_ROUTE = /^\/signal\/v1\/rooms\/([A-Za-z0-9_-]{22})\/(host|join)$/;

export async function onRequest(context: { request: Request; env: PagesEnv }): Promise<Response> {
  const { request, env } = context;
  const path = new URL(request.url).pathname;
  if (path === "/signal/v1/health") {
    return Response.json(env.ROOMS ? { ok: true, protocol: 1 } : { ok: false, protocol: 1 }, { status: env.ROOMS ? 200 : 503 });
  }
  const m = ROOM_ROUTE.exec(path);
  if (!m) return new Response("not found", { status: 404 });
  if (!env.ROOMS) return new Response("the signaling service is not configured on this site", { status: 503 });
  const ns = env.ROOMS;
  return ns.get(ns.idFromName(m[1] as string)).fetch(request);
}
