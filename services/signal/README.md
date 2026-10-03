# Ethereal signaling service

The only hosted piece of Ethereal's sharing (design: [docs/SHARING.md](../../docs/SHARING.md)).
A Cloudflare Worker with one Durable Object per room. It introduces a joiner to the host's app
and forwards their WebRTC offer/answer/ICE. After that the peers talk directly (DTLS-encrypted
data channel). It never sees a project, an op, a link key or a member key.

Status: **skeleton** (base-115). Routes, origin checks, message validation and the wire types
are in place and tested. The room logic (`RoomCore`) is implemented by the `signal-service`
node.

## API (v1)

| Route | What |
|---|---|
| `GET /v1/health` | `{"ok":true,"protocol":1}` (CORS for allowed origins) |
| `GET /v1/rooms/<room>/host` (WebSocket) | the host's socket; first frame `HostHello` |
| `GET /v1/rooms/<room>/join` (WebSocket) | a joiner's socket; first frame `JoinHello` |

The same routes are served under `/signal/...` when the service is mounted on the web app's
origin (the Pages Function proxy below).

The messages are JSON text frames, typed by `SignalClientMessage` / `SignalServerMessage`. They
are generated from Rust (`crates/ether-protocol/src/share.rs`, `just gen-types`) and imported
from `ui/src/generated` as types only. See docs/SHARING.md §3 for the state machine and the
limits (`src/protocol.ts` `LIMITS`).

What a room stores: SHA-256 of the host token, SHA-256 of each door (one per link and per
member), and when the host last connected. Nothing else is stored, and signals are never
stored. A room is deleted when the host sends `CloseRoom` ("Stop sharing"), or `ROOM_TTL_DAYS`
(default 30) after its host last connected.

## Develop

```sh
pnpm --filter @ethereal/signal test        # unit tests (no Workers runtime needed)
pnpm --filter @ethereal/signal typecheck
npx wrangler dev                           # local Worker + Durable Object (needs wrangler)
```

## Deploy (manual, owner only; not part of CI)

1. `npx wrangler login`, then `npx wrangler deploy` from this folder. That creates the
   `ethereal-signal` Worker and its `SignalRoom` Durable Object class (SQLite-backed, free plan
   compatible).
2. Mount it on the web app's origin, so the default `https://etherealws.pages.dev/signal`
   works. There is no extra domain and no CORS under COEP. Add a Pages Function to the web
   deploy, `functions/signal/[[path]].ts`, that forwards to the Durable Object through a
   binding (Pages project → Settings → Bindings → Durable Object `ROOMS` → script
   `ethereal-signal`, class `SignalRoom`). The `signal-service` node ships this file.
   Otherwise, set the Worker's URL in the app (Settings > Advanced > Signaling server). Links
   then carry it as `?s=`.
3. Optional TURN: `wrangler secret put TURN_KEY_ID` and `TURN_KEY_API_TOKEN` (Cloudflare
   Realtime TURN). The service then adds short-lived TURN credentials to the ICE servers it
   advertises.

## Self-hosting

Any Cloudflare account works: deploy as above, then point the app at it in Settings >
Advanced. Invite links made by that app carry `?s=<your service>`, so joiners use it too. The
service is about 300 lines with no dependencies. A Node/Deno port only needs a WebSocket server
and a map of rooms, because `RoomCore` has no Workers-specific code.

## Limits (see `src/protocol.ts`)

- 64 KiB per frame, 32 KiB per SDP, 1 KiB per candidate, 50 frames/s per socket.
- 32 joiner sockets per room, 66 doors per room, 200 signals per pairing.
- A joiner waits at most 10 minutes for an offline host (then retries with backoff).
- Per client IP: 20 bad doors/min and 30 room claims/hour (Workers rate-limiting binding or a
  per-colo counter). Exceeding them returns `Refused { RateLimited }`.
