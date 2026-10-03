# Ethereal signaling service

The only hosted piece of Ethereal's sharing (design: [docs/SHARING.md](../../docs/SHARING.md) §3).
A Cloudflare Worker with one Durable Object per room. It introduces a joiner to the host's app
and forwards their WebRTC offer/answer/ICE. After that the peers talk directly (DTLS-encrypted
data channel). It never sees a project, an op, a link key or a member key.

## Layout

| File | What |
|---|---|
| `src/protocol.ts` | routes, `LIMITS`, origin check, validation of every inbound frame |
| `src/room.ts` | `RoomCore`: the room state machine (§3.3). No Workers code, so it runs anywhere |
| `src/do.ts` | the Durable Objects: `SignalRoom` (one per room, WebSocket hibernation) and `IpBudget` (one per client IP) |
| `src/index.ts` | the Worker: health, origin check, forwards upgrades to the room |
| `src/ice.ts` | the ICE servers advertised in welcomes: `STUN_URLS` + optional TURN |
| `src/budget.ts` | per-IP sliding windows (claims, bad doors) |
| `test/server.ts` | the same service on Node (`ws`), for e2e tests and local dev |
| `test/smoke.ts` | end-to-end smoke test against any running instance |
| `../../scripts/release/web/functions/signal/[[path]].ts` | the Pages Function that mounts the service at `<web app>/signal` |

## API (v1)

| Route | What |
|---|---|
| `GET /v1/health` | `{"ok":true,"protocol":1}` (CORS for allowed origins) |
| `GET /v1/rooms/<room>/host` (WebSocket) | the host's socket; first frame `HostHello` |
| `GET /v1/rooms/<room>/join` (WebSocket) | a joiner's socket; first frame `JoinHello` |

The same routes are served under `/signal/...` (the Pages Function, and the Node adapter).

The messages are JSON text frames, typed by `SignalClientMessage` / `SignalServerMessage`. They
are generated from Rust (`crates/ether-protocol/src/share.rs`, `just gen-types`) and imported
from `ui/src/generated` as types only.

- **Doors**: the host sends, and the room stores, the lowercase hex SHA-256 of each door's
  base64url text (the 22 characters a joiner sends in `JoinHello.door`). The host token is
  stored the same way (SHA-256 of its 43-character base64url text).
- **Peers**: a joiner gets its pairing id in `JoinWelcome.peer`; the host learns it from
  `PeerArrived`. Signals from a joiner are stamped with its own id, whatever it wrote.
- **Close codes**: `4000` after `Refused { reason }` (the close reason is the
  `SignalRefusal`), `4001` host socket replaced by a newer one with the same token, `4002` no
  hello within 10 s or the host stayed offline for 10 min (retry with backoff), `4003`
  `EndPeer` from the host, `1000` `CloseRoom`.

## Room state machine (docs/SHARING.md §3.3)

- **Claim**: the first `HostHello` stores SHA-256(`host_token`) and the doors. Later ones must
  present the same token (`NotHost` otherwise). A second host socket with the right token
  replaces the first (the app restarted); joiners are introduced to the new one.
- **Join**: a valid door gets `JoinWelcome` (and the host `PeerArrived`) when the host is
  online, else `HostOffline { last_seen_ms }` and a wait of up to 10 minutes. An unknown room
  and a wrong door both get `Refused { InvalidInvite }`.
- **Host leaves**: every joiner, paired or not, gets `HostOffline` and waits again.
- **Signals** are forwarded between the host and a paired joiner, at most 200 per pairing.
  `EndPeer` (host) or either socket closing ends a pairing (`PeerLeft` to the host).
- **SetDoors** replaces the doors; joiners whose door was removed are refused at once.
- **CloseRoom** deletes the record and closes every socket. The **TTL** alarm deletes a room
  `ROOM_TTL_DAYS` after its host was last connected.

What a room stores: SHA-256 of the host token, SHA-256 of each door, and when the host last
connected. Signals are never stored or logged.

## Limits (`src/protocol.ts` `LIMITS`)

- 64 KiB per frame, 32 KiB per SDP, 1 KiB per candidate, 50 frames/s per socket.
- 32 joiner sockets per room (waiting, paired or before their hello), 66 doors per room, 200
  signals per pairing. The first frame must arrive within 10 s.
- A joiner waits at most 10 minutes for an offline host (then retries with backoff).
- Per client IP (`IpBudget` objects): 20 bad doors per minute, then even valid doors are
  `RateLimited` from that address for the rest of the minute; 30 room claims per hour.

## Develop

```sh
pnpm --filter @ethereal/signal test        # unit + Node adapter tests (no Workers runtime needed)
pnpm --filter @ethereal/signal typecheck
pnpm --filter @ethereal/signal dev:node    # the service on Node: http://127.0.0.1:8789
```

### `wrangler dev` smoke test

Wrangler is not a dependency; `npx` fetches it.

```sh
cd services/signal
npx wrangler@4 dev --port 8787             # local Worker + Durable Objects (workerd)
# in another terminal:
node test/smoke.ts http://127.0.0.1:8787   # prints the steps, then "smoke: ok"
```

The smoke test checks health, the claim, a wrong door, an introduction with an offer and an
answer, `EndPeer`, a joiner waiting for an offline host, `PeerLeft` and `CloseRoom`. Run it
against the Node adapter (`node test/smoke.ts http://127.0.0.1:8789`) or a deployment
(`node test/smoke.ts https://etherealws.pages.dev/signal`) the same way. Local state lives in
`.wrangler/` (ignored); delete it to start clean.

### In e2e tests

```ts
import { startSignalServer } from "../../services/signal/test/server.ts";
const signal = await startSignalServer({ port: 0 }); // any Origin, no ICE servers, no per-IP budgets
// ... use signal.url as the signaling URL ...
await signal.close();
```

Or as a process (Playwright `webServer`): `node services/signal/test/server.ts --port <port>`
prints `signal: http://127.0.0.1:<port>` once listening. Node ≥ 23.6 runs the TypeScript
directly.

## Deploy (manual, owner only; not part of CI)

1. `npx wrangler login`, then `npx wrangler deploy` from this folder. That creates the
   `ethereal-signal` Worker and its `SignalRoom` and `IpBudget` Durable Object classes
   (SQLite-backed, free plan compatible). Check it: `node test/smoke.ts
   https://ethereal-signal.<account>.workers.dev`.
2. Mount it on the web app's origin, so the default `https://etherealws.pages.dev/signal`
   works (no extra domain, no CORS under COEP):
   - the web deploy must include `scripts/release/web/functions/` as the Pages project's
     `functions/` folder (run `wrangler pages deploy <site>` from `scripts/release/web`, or
     copy the folder next to the site);
   - Pages project → Settings → Bindings → Durable Object: name `ROOMS`, Worker
     `ethereal-signal`, class `SignalRoom`.

   Then `node test/smoke.ts https://etherealws.pages.dev/signal`. Without the binding the
   function answers 503 and the app reports the service as offline. Alternatively, set the
   Worker's URL in the app (Settings > Advanced > Signaling server). Links then carry it as
   `?s=`.
3. Optional TURN: `wrangler secret put TURN_KEY_ID` and `TURN_KEY_API_TOKEN` (Cloudflare
   Realtime TURN). The service then adds short-lived TURN credentials (24 h, reused for an
   hour) to the ICE servers it advertises. Without them it advertises STUN only.

## Self-hosting

Any Cloudflare account works: deploy as above, then point the app at it in Settings >
Advanced. Invite links made by that app carry `?s=<your service>`, so joiners use it too.
Without Cloudflare, `test/server.ts` is a complete Node implementation (rooms in memory): run
`node test/server.ts --port 8789 --stun stun:stun.cloudflare.com:3478 --ip-budgets
--trust-proxy` behind a TLS reverse proxy that sets `X-Forwarded-For`.
