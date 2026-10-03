# Sharing: P2P host hub, invite links (base-115)

Status: **implemented** (the owner accepted the design in #198; nodes in §12, integration
and end-to-end tests in §12.1). The default service runs at
`https://etherealws.pages.dev/signal` (§3.1, §6.3). It builds on
[COLLAB.md](COLLAB.md): the replicated document, sequenced op log, per-site undo, presence,
chat and listen-on-peer all stay as they are. What changes is how sites reach each other, who
sequences, and how people are invited. Contract: CONTRACTS.md §11.18. Nodes: ROADMAP.md
"Sharing (base-115)". Remaining choices are listed in §13.

## 0. Owner decisions (final) and what they mean here

| Decision | Consequence in this design |
|---|---|
| **P2P transport** (WebRTC data channels), a tiny hosted signaling service, TURN optional | `services/signal/`: a Cloudflare Worker + one Durable Object per room. It only forwards SDP/ICE. Peers then talk over a DTLS data channel. ICE servers come from the service (STUN by default, TURN if configured) or from Settings (§3, §6) |
| **Anonymous identity** (name + colour). **Invite links** carry the capability (edit or listen). Account-ready | A link = room id + secret key (in the URL fragment). A joiner proves the key to the host, bound to the DTLS fingerprints, and gets a per-member key for later rejoins. Accounts later replace the key with an account proof in the same slots (§4) |
| **The host is the master** (star; the host orders and relays ops). Collaborators keep a synced copy that becomes an offline copy. Reopening reconnects. Host offline = no join | The hub **is today's relay state machine** (`ether_collab::relay::Relay`, wasm-safe), run inside the sharer's app. The host's own site joins it over an in-process loopback link, and every joiner over a data channel carrying the **unchanged** `CollabMessage` wire. The controller's collab code (rebase, undo, catch-up, the "(local copy)" logic) is reused as is (§2) |
| **UX**: Share button → link → "X invites you to Song: Join?", Recents badges and avatars, one session pill, Remote engine in Settings > Advanced | §8 |

## 1. Vocabulary

- **Host**: the app that shares the project. It runs the **hub**. Its copy is the master.
- **Joiner / collaborator / member**: an app that joined through a link. A **member** is a
  joiner the host remembers (it holds a member key and can rejoin even after the link is reset).
- **Room**: the signaling service's rendezvous for one shared project (`RoomId`, 16 random
  bytes).
- **Pairing**: one joiner socket on the signaling service (`PeerId`). It becomes one WebRTC
  peer connection with one data channel.
- **Offline copy**: a collaborator's stored copy of a shared project, with its `share.json`
  (§6.4).

## 2. Topology and roles

```
                      signaling service (CF Worker + Durable Object per room)
                      only: room claim, doors, SDP/ICE forwarding, host online?
                     ╱ wss (host socket)        ╲ wss (one per joiner, until paired)
                    ╱                             ╲
  ┌──────────────── Host app (Diego) ─────────────┐        ┌──── Joiner app (Ada) ────┐
  │ controller ── loopback link ──► HUB           │  DTLS  │ controller (site A)      │
  │  (site D)                      (Relay state   │◄══════►│  collab session over a   │
  │                                 machine: seq, │  data  │  PeerLink (CollabMessage │
  │                                 fan-out, log, │ channel│  frames, fragmented)     │
  │                                 catch-up,     │        └──────────────────────────┘
  │                                 roles filter) │◄══════► Joiner app (Tom, listen link)
  └───────────────────────────────────────────────┘
        listen-on-peer audio: separate WebRTC streams as today (COLLAB.md §9),
        signalled with CollabMessage::Signal through the hub instead of the relay
```

- **Star**: every joiner has exactly one data channel, to the host. The hub sequences every
  transaction (one total order), fans out, keeps the log, the latest snapshot and the media
  cache for late joiners and reconnects, assigns colours and stamps presence. In short, it
  does exactly what `ether-collab-relay` does today (COLLAB.md §1-§7), in-process.
- **The host is also a site**: its controller joins the hub like any site (`Hello`,
  `SyncRequest`). It is the first site, so the hub asks it to create the session (media and
  a snapshot of its open project). Its edits go through the same pending/echo path as
  everyone's. On a loopback link the echo is immediate, so nothing changes for the user.
- **Joiners never talk to each other.** Site-to-site messages (listen requests, WebRTC
  signals for audio streams, transport requests, stream clocks) are routed by the hub
  (`CollabMessage::route`), as the relay routes them today.

### 2.1 Why the relay maps onto WebRTC with almost no change

| Today (relay) | Sharing (hub) | Code change |
|---|---|---|
| `ether-collab-relay` process; `relay::server` drives `Relay` over WebSockets | `ether_collab::share::hub` (node `share-engine`) drives the **same** `Relay` over `PeerLink`s and one loopback link | new driver (≈ the existing `memory::Hub`, with auto-delivery) |
| site → relay: `CollabTransport` = `native::WsClient` / `web::WebClient` | site → hub: a `CollabTransport` over a `PeerLink` (joiner) or the loopback (host) | new transports. The controller picks them through its existing `Connector` seam |
| WebSocket text/binary frames (`wire::encode_frame`) | the same `WireFrame`s, cut into ≤ 16 KiB data-channel messages (`share::dc`) | `dc.rs` (frozen, implemented) |
| handshake `ClientHello{token}` / `ServerHello` | `PeerHandshake::{Hello, Welcome, Accept, Refused}` with key proofs (§4.3) | new, in the transport, before any `CollabMessage` |
| relay-wide token | per-link / per-member keys and roles | hub role filter (§2.3) |
| relay advertises `IceServers` (its own STUN/TURN) | the hub advertises the signaling service's ICE servers (or Settings) | `Relay::set_ice_provider` with that list |
| reconnect: controller backoff → `connect(url)` again | the same backoff calls the share connector, which runs signaling + ICE + handshake again (member key) | none in `collab/mod.rs` |
| session name in the URL | fixed `"share"` session inside the hub (one per hub) | none |

So `crates/ether-controller/src/collab/**` keeps its algorithms. The share module only adds
three small seams there (owned as shared touches by `share-engine`): pick the connector per
session, seed `left_sites` from `share.json` (§7.1), and expose the confirmed-sync and
catch-up-end signals that `ShareState` needs.

### 2.2 Data channel

- One data channel per pairing, label `ethereal-collab/1` (`share::dc::DC_LABEL`), ordered
  and reliable (SCTP). The joiner creates it and sends the offer. The host answers. A passive
  host scales and lets str0m/browser hosts work the same.
- Every `WireFrame` (JSON text, or the binary media frame of `wire.rs`) is cut into data
  channel messages of ≤ `DC_FRAGMENT_BYTES` (16 KiB): `[flags u8][bytes]`, bit 0 = last
  fragment, bit 1 = binary frame. Browsers cap message sizes (Safari ~64 KiB), and a 16 MiB
  snapshot in one message would stall the stream. Reassembled frames are capped at
  `MAX_COLLAB_MESSAGE_BYTES` (16 MiB), as today. Implemented and tested in
  `crates/ether-collab/src/share/dc.rs`.
- **Backpressure**: `PeerLink::buffered()` (`bufferedAmount` on web, str0m's SCTP send
  buffer natively). The hub treats a link past its byte budget like the relay treats a slow
  reader (`RelayServerConfig::max_queued_bytes`: closed, then the joiner resyncs on
  reconnect). Senders pace media chunks on `bufferedAmountLow`.

### 2.3 Roles in the hub

The hub knows each connection's role from the handshake (`Edit`, `Listen`; the host is
`Host`). Before `Relay::message` it filters:

| Message from a `Listen` connection | Hub |
|---|---|
| `Hello`, `SyncRequest` (resume or full), `Presence`, `Pointer`, `Leave`, `Listen`, `Unlisten`, `Signal`, `TransportRequest` | passed |
| `Transaction` with only chat ops (`Insert ChatMessage` + its prune `Remove`s; the §12.1 sanitize rules still apply) | passed (listeners can chat) |
| any other `Transaction`, `Media`, `Snapshot`, `Update` | dropped and the link is closed (a well-behaved controller never sends them, see below) |

- The hub never asks a `Listen` connection for a snapshot (session creation is always the host;
  compaction asks the host). This needs a small `Relay` hook, `set_snapshot_source(conn)`,
  a shared touch of `share-engine` on `relay/mod.rs`.
- The hub rewrites `Hello.name` to the authenticated handshake name and holds each site id
  on its connection, so a joiner cannot impersonate another site. The relay already
  enforces `origin.site == Hello site`.
- Colours: the joiner's preferred colour (handshake) when it is free, else the first free
  `PEER_COLORS` entry. This needs a `Relay::set_color(conn, color)` hook (same shared touch).
- On the joiner, the controller refuses document commands in a `Listen` session
  (`InvalidState`, "view only: you joined with a listen link"). The UI greys out edit
  affordances and shows a "View only" chip (§8.4). After sync, a listen joiner starts
  `Listen { host }` automatically: it hears the host's mix (COLLAB.md §9). It can stop
  listening and play its read-only copy locally.
- **Role changes** (`SetParticipantRole`): the host closes that link. The member rejoins at
  once with its member key and gets the new role in `Welcome`.

### 2.4 ICE servers and the listen streams

- The signaling service advertises ICE servers in `HostWelcome`/`JoinWelcome` (default:
  `stun:stun.cloudflare.com:3478`; TURN credentials if the operator configured them, §3.4).
  Settings > Advanced overrides them (`CollabCommand::SetIceServers`, unchanged). The
  `iceTransportPolicy: "relay"` option ("Hide my IP: relay only") applies when TURN exists.
- The hub installs that list as its `Relay::set_ice_provider`, so every site keeps receiving
  `CollabMessage::IceServers` exactly as from a relay. The **listen-on-peer audio streams
  (`stream-host`/`stream-listen`, COLLAB.md §9) work unchanged**: their `Listen`/`Signal`/
  `StreamClock` messages travel through the hub. The native sender still runs its own str0m
  `Rtc` per listener on its UDP socket, and the browser receiver its own
  `RTCPeerConnection`. One extra ICE negotiation per listener is negligible.
- Future (not needed): add the Opus track to the existing data-channel peer connection
  (renegotiation) to save that ICE round and a port.

## 3. Signaling service (`services/signal/`)

### 3.1 Hosting

- A Cloudflare **Worker** (`services/signal/src/index.ts`) plus one **Durable Object per room**
  (`SignalRoom`, `src/room.ts`, WebSocket hibernation API, SQLite-backed storage, free-plan
  compatible). The room logic is a pure state machine (`RoomCore`) that is unit-tested
  without the Workers runtime and is portable to Node/Deno for self-hosting.
- **Default URL `https://etherealws.pages.dev/signal`**: a Pages Function
  (`functions/signal/[[path]].ts` in the web deploy) forwards to the Durable Object through a
  DO binding. So the service is **same-origin with the web app**: no CORS preflight, no
  `Cross-Origin-Resource-Policy` problems under the app's COEP, and no second domain to
  manage. A standalone `*.workers.dev` URL also works (Settings > Advanced, links carry
  `?s=`). Decision 2 in §13.
- Not deployed by CI. `README.md` has the manual `wrangler deploy` steps (owner only).

### 3.2 API (v1)

| Route | Frames |
|---|---|
| `GET /v1/health` | `{"ok":true,"protocol":1}` |
| `GET /v1/rooms/<room>/host` (WebSocket) | client: `HostHello`, `SetDoors`, `Signal`, `EndPeer`, `CloseRoom`, `Ping`. Server: `HostWelcome`, `PeerArrived`, `PeerLeft`, `Signal`, `Refused`, `Pong` |
| `GET /v1/rooms/<room>/join` (WebSocket) | client: `JoinHello`, `Signal`, `Ping`. Server: `JoinWelcome`, `HostOffline`, `Signal`, `Refused`, `Pong` |

The messages (`SignalClientMessage`, `SignalServerMessage`, `SignalRefusal`) are defined in
`crates/ether-protocol/src/share.rs`, generated to `ui/src/generated`, and imported by the
Worker as types (`services/signal/src/protocol.ts` validates every inbound frame). Secrets
never travel in URLs: the door and host token are in the first frame.

### 3.3 Room state machine

Persisted per room, and nothing else: `hostTokenHash`, `doors[]` (SHA-256 hex),
`lastHostSeenMs`.

- **Claim**: the first `HostHello` on an unclaimed room stores SHA-256(`host_token`) and
  the doors (trust on first use: room ids are 128-bit random and chosen by the host). Later
  `HostHello`s must present the same token (constant-time compare of hashes), else
  `Refused{NotHost}`. A second host socket with the right token replaces the first, which
  is closed: the app restarted.
- **Host online**: `HostWelcome{ice_servers, room_ttl_s}`. Every joiner socket waiting
  (`HostOffline`) is welcomed now (`JoinWelcome{peer}` to it, `PeerArrived{peer}` to the host).
- **Join**: `JoinHello{door}` → SHA-256(door) ∈ doors? Otherwise `Refused{InvalidInvite}`.
  An unknown room gets the same answer, so room ids cannot be enumerated. The per-IP
  bad-door budget applies. When the host is online, the joiner gets `JoinWelcome` and the
  host gets `PeerArrived`. Otherwise the joiner gets `HostOffline{last_seen_ms}` and its
  socket stays open (≤ 10 min, then closed; the client retries with backoff).
- **Signals**: `Signal{peer, signal}` from the host goes to that joiner. From a joiner, the
  service stamps the joiner's own `peer` and sends it to the host. At most 200 signals per
  pairing. A pairing ends with `EndPeer` from the host or when either socket closes
  (`PeerLeft`). Once the data channel is up, the joiner closes its signaling socket, so
  sockets only exist during introductions.
- **SetDoors** (link reset, member added/removed) replaces the doors. **CloseRoom** ("Stop
  sharing") deletes the record and closes every socket. **TTL**: an alarm deletes the room
  `ROOM_TTL_DAYS` (30) after the host was last connected.
- Stateless otherwise: no project data, names, SDP history or logs of signals. A restart of
  the Durable Object loses only open sockets, and clients reconnect.

### 3.4 Abuse limits, CORS, TURN

- Frames ≤ 64 KiB, SDP ≤ 32 KiB, candidate ≤ 1 KiB, 50 frames/s per socket (token bucket),
  hello within 10 s, ≤ 32 joiner sockets per room, ≤ 66 doors, ≤ 200 signals per pairing.
  Per client IP (Workers rate-limiting binding): ≤ 20 bad doors/min, ≤ 30 room claims/hour.
  Malformed frames close the socket (`Refused{Malformed}`).
- **Origins**: browsers must present an allowed `Origin` (`ALLOWED_ORIGINS`: the Pages
  origin and its preview subdomains, the Tauri webview origins, `localhost` for dev). Native
  clients send none and are allowed. This is not authentication (non-browser clients can lie).
  It keeps other websites from using the service through their visitors' browsers. HTTP
  routes send `Access-Control-Allow-Origin` for allowed origins only.
- **TURN (optional)**: with `TURN_KEY_ID`/`TURN_KEY_API_TOKEN` secrets (Cloudflare Realtime
  TURN), the service mints short-lived TURN credentials per welcome and adds them to
  `ice_servers`. Without them it advertises STUN only. Self-hosters can point the app at
  their own coturn through Settings, or run the relay's experimental `--turn`
  (COLLAB.md §10) on a VPS.
- **Self-hosting**: deploy `services/signal` to any Cloudflare account (README), set
  Settings > Advanced > Signaling server. Links created by that app carry `?s=<url>`, so
  joiners use the same service without configuration.

## 4. Invite links and capabilities

### 4.1 Format (frozen; `ether_collab::share::invite`, `ui/src/domain/invite.ts`)

```text
https://etherealws.pages.dev/join/<room>[?s=<signal url>]#<key>
ethereal://join/<room>[?s=<signal url>]#<key>
```

- `room`: 16 random bytes, base64url (22 chars), stable while the project is shared.
- `key`: version char + secret. `1` = v1 link key, 16 random bytes base64url (22 chars), so
  a link is ≈ 85 characters. **The key is in the fragment**, which browsers never send to a
  server: neither Pages nor the signaling service ever see it (decision 1 in §13). The web
  app reads it from `location.hash`, and the desktop app gets it in the deep-link string.
- `s`: non-default signaling service (percent-encoded), for self-hosting.
- **Account-ready**: a link **without a key** parses as a bare room reference. It is refused
  today (`BadLink`) and reserved for "join as my account". Key versions other than `1` are
  refused with "needs a newer version" (`UnknownKeyVersion`), so a later format never
  breaks silently. In the handshake, `Credential::{Link, Member{member}}` gets a third
  variant (`Account{token}`), and `MemberId` becomes the account's id. Nothing else moves.

### 4.2 Secrets and what each party holds

| Secret | Who holds it | Derived values (HMAC-SHA256, `K` = key bytes) | Who sees the derived value |
|---|---|---|---|
| link key `K_edit`, `K_listen` (per shared project, rotatable) | host (`share.json`), anyone with the link | `door = HMAC(K, "ethereal/share/v1/door" ‖ room)[..16]` (base64url) | signaling service (as SHA-256(door) from the host; the door itself from joiners) |
| member key `K_m` (per member, issued at first join) | host and that member (`share.json`) | its door, the same way | signaling service |
| host token (32 random bytes) | host only | SHA-256 | signaling service |

- **Doors** keep strangers from reaching the host: the service relays an introduction only
  for a valid door. The service learns doors, not keys. A door cannot be turned into a key
  or a proof.
- **Proofs** (§4.3) are computed over the **DTLS fingerprints** of the actual connection, so
  a malicious or compromised signaling service, which could swap SDP fingerprints and sit in
  the middle, cannot produce or relay a valid proof. The key never crosses the network, not
  even encrypted.

**Frozen test vectors** (`ether_collab::share::keys::tests::frozen_vectors`). `K` = bytes
`00..0f` (secret `AAECAwQFBgcICQoLDA0ODw`, link key `1AAECAwQFBgcICQoLDA0ODw`), room
`AbCdEfGhIjKlMnOpQrStUv`, `fp_joiner = "sha-256 AA:BB:CC"`, `fp_host = "sha-256 11:22:33"`.
Strings are concatenated as UTF-8 without separators; outputs are base64url without padding.
- door = `-NlMpoJxAQr27xLrrnWcyg`; door hash (lowercase hex SHA-256 of the door string) =
  `0ca35b191565d6d1aa58791582901a42b1ad1a79af44ae854d4b7c4b7dd63b28`
- join proof = `IIhhbob-mfbMMQj0IERpl9I5YvYUsGRkYgAGYT4B3xs`
- host proof = `Em9eKyuWCBC-Dygl1V2JRSJn5rlsPhSqcQ_mp-knftk`

(Cross-checked by share-engine against an independent Python `hmac`/`hashlib` computation.)

### 4.3 Handshake on the data channel (`PeerHandshake`, before any `CollabMessage`)

```
joiner → host  Hello   { share_protocol, collab_protocol, credential: Link | Member{member},
                         proof = HMAC(K, "ethereal/share/v1/join" ‖ fp_joiner ‖ fp_host),
                         name, color, app }
host           tries K_edit, K_listen (Link) or K_m (Member) in constant time → role
               (none matches → Refused{InvalidInvite}; versions differ → Refused{Version};
                session full → Refused{Refused})
host → joiner  Welcome { host_proof = HMAC(K, "ethereal/share/v1/host" ‖ fp_host ‖ fp_joiner),
                         role, member, member_key (fresh on a Link join), host, project,
                         project_name, online }
joiner         verifies host_proof (else JoinFailure::HostNotVerified, never joins)
               → the join screen shows "<host> invites you to <project>: Join?" (first join),
                 or continues straight away (member rejoin)
joiner → host  Accept   → both sides switch to CollabMessage frames (Hello, SyncRequest, ...)
```

- `fp_*` = the `a=fingerprint` value of each side's DTLS certificate as it appears in SDP
  (`"sha-256 AB:CD:..."`, uppercase hex), local and remote (`PeerOutput::Connected`).
- The project name and the participants are sent **only to an authenticated joiner over the
  encrypted channel**: the signaling service never learns them. The cost is ~1-2 s of
  "Connecting…" before the join screen can name the host and project (§8.2).
- The host records a new member only on `Accept` (declined invites leave no trace).
  Members: ≤ `MAX_MEMBERS` (64). Participants online: ≤ `MAX_PARTICIPANTS` (16, the relay's
  `max_sites_per_session`).

### 4.4 Revocation

- **Reset link** (`ResetLink{role}`): new `K` for that role, `SetDoors`. The old link stops
  working at once. **Members already admitted keep access** (their member keys are separate)
  (decision 7).
- **Remove** (`RemoveParticipant{member}`): close its link, delete its member record,
  `SetDoors`. It can come back only through a current link.
- **Stop sharing** (`Stop`): `CloseRoom` (the service forgets the room), every link closed,
  `share.json` deleted (or downgraded to "not shared"). All links and member keys are dead,
  and a later Share creates a new room and new links. Quitting the app is **not** stopping
  (§7.1).

### 4.5 Where secrets live: `share.json`

The sharing state of a project is stored next to its `project.ether`, in `share.json`
(`ether_collab::share::file`, frozen, versioned). It is written through the store's
existing file API, so the same code works natively, on OPFS and in memory.
- **Host** (`ShareFile::Host`):
  - `room`, `signal_url`, `host_token`;
  - `edit_key` / `listen_key` (`None` = that link is off);
  - `members[]` (member id, key, role, name, colour, last seen);
  - `sites` (last sequenced `seq` per site, seeding the hub after a restart);
  - `resume` (sharing was on when the project closed).
- **Copy** (`ShareFile::Copy`): `room`, `signal_url`, `member`, `key` (the member key), `role`,
  `host_name`, last known `participants`, `last_synced_ms`.
- It is **never** in the replicated document, a snapshot, a media push, an export or a
  duplicate: `SaveAs`/`Duplicate` skip it, and deleting the project deletes it. Recents reads
  it (role, host name, avatars, `active`) into `ProjectSummary.share`. The keys never reach
  the UI.

## 5. Desktop deep links and the web `/join` route (`join-flow`)

- **Desktop**: `tauri-plugin-deep-link` registers `ethereal://` (bundle config:
  `plugins.deep-link.desktop.schemes = ["ethereal"]`; macOS Info.plist URL types, Windows
  registry and Linux `.desktop` MimeType are handled by the plugin/bundler).
  `tauri-plugin-single-instance` (feature `deep-link`) hands the URL to the running app
  instead of starting a second one. The shell emits a `deep-link` event to the webview. The
  UI routes `ethereal://join/...` to the join screen, which sends `Share::OpenInvite{link}`.
  The link the UI receives at cold start (`getCurrent()`) is handled the same way. Dev
  instances (`ETHER_INSTANCE`) do not register the scheme. Workspace deps were added in this
  PR.
- **Web route**: Pages serves the SPA for any path (no `404.html` in the web build: the
  `join-flow` node checks and adds a `_redirects` rule `/join/* /index.html 200` if needed).
  `apps/web/src/main.tsx` checks `location.pathname` for `/join/` **before booting the
  engine** and renders the lightweight **join landing**:
  - The host's name is not known yet (it comes from the host after authentication, §4.3),
    so the landing says "You've been invited to an Ethereal project" with two buttons:
    **Open in the app** (`ethereal://join/<same tail>`; if the page is still visible after
    1.5 s, it shows "Don't have the app? Download it" next to the button) and **Continue in
    browser** (boots the web app, then the join screen). The choice can be remembered
    ("Always continue in browser", `localStorage`). Phones go straight to the browser.
  - The URL is rewritten to `/` (the fragment dropped) with `history.replaceState` once the
    engine has the invite, so a reload or a shared screenshot does not leak the key.
- **Pasting a link**: the "Join with a link…" dialog accepts any invite form
  (`parseInvite`). It opens from the command palette ("Join shared project…") and from the
  Share popover's "…" menu ("Join with a link…").

## 6. Native WebRTC and the web endpoint (`p2p-transport`)

### 6.1 Choice: **str0m in the engine process** (desktop and `ether-server`)

| Option | For | Against |
|---|---|---|
| **str0m (chosen)** | Already in the tree for `stream-host` (`str0m 0.24`, `rust-crypto`, which pulls `sctp-proto`). Data-channel support adds no new crate. Sans-IO and sync: one thread, one UDP socket, deterministic tests with fake time. The session lives next to the controller and the hub, so it does not depend on the webview (Linux WebKitGTK builds without WebRTC can host and join). Headless `ether-server` can host (an always-on studio box) | We own ICE gathering (host + srflx, the `stream/stun.rs` pattern) and str0m has no TURN client (§11). str0m's SCTP is younger than libwebrtc's |
| webrtc-rs | Full stack incl. TURN client | tokio everywhere, a large tree (+3-5 MB), API churn, moving to a sans-IO rewrite. Already rejected in COLLAB.md §11 |
| Data channel in the webview, bridged to the engine | Same code path as web, and libwebrtc's maturity in Chromium-based webviews | Every op and media chunk crosses Tauri IPC as JSON (base64 media, 16 MiB snapshots). The session dies with the webview. No WebRTC in some WebKitGTK builds. `ether-server` cannot host |

- **Bundle size**: data-channel use adds roughly +150-300 KB to the desktop binary (str0m's
  SCTP and data-channel code, already compiled in). The signaling client needs TLS: the
  `tungstenite` `rustls-tls-webpki-roots` feature (rustls + `ring`, already in the lockfile;
  roughly +1 MB). This also gives the relay client `wss://`, closing a known limitation in
  COLLAB.md §13 (decision 6).
- **Maintenance**: one WebRTC stack natively (str0m) instead of two. Its upgrades already
  happen for `stream-host`. Native ICE is shared with `ether-native/src/stream/stun.rs` (host
  candidates + one srflx Binding per STUN URL). It lives in
  `crates/ether-collab/src/share/native/**` (thread + socket), and `ether-collab` gains
  `str0m` as a native-only dependency.
- Threading: one `ether-share` thread owns the UDP socket and every pairing's `Rtc`. It
  exchanges `PeerOutput`s/commands with the controller over `crossbeam-channel`s (non-blocking
  `PeerEndpoint::poll`), the same pattern as the stream sender.

### 6.2 Web: the UI's `RTCPeerConnection`, bytes on a `MessagePort`

`RTCPeerConnection` does not exist in Workers, and the controller and hub run in the
controller Worker. So the UI thread owns the peer connections and the Worker drives them
over one `MessagePort`, the **share port** (as implemented by `p2p-transport`):

- At startup the page (`apps/web/src/engine/endpoint.ts`) creates a `MessageChannel`, keeps
  one end for the UI agent (`ui/src/features/share/endpoint/**`) and transfers the other to
  the controller Worker, which installs it (`ether_wasm::install_share_port` →
  `ether_collab::share::web::install_port`).
- **Everything for a pairing travels on that port**, not in the JSON engine protocol: the
  open/close requests, the SDP/ICE signals both ways, the data-channel bytes and the
  backpressure counters. Each pairing gets a channel number `ch` chosen by the Worker, so a
  replaced pairing never receives the old one's messages:

  | Worker → UI | UI → Worker |
  |---|---|
  | `{type:"open", ch, offer, ice, relay}` | `{type:"signal", ch, signal}` (`StreamSignal`) |
  | `{type:"signal", ch, signal}` | `{type:"open", ch, local, remote}` (DTLS fingerprints from the SDPs) |
  | `{type:"data", ch, data: ArrayBuffer}` (one ≤ 16 KiB `dc` fragment, transferred) | `{type:"data", ch, data}` |
  | `{type:"close", ch}` | `{type:"flow", ch, consumed, buffered}`, `{type:"closed", ch, reason}` |

- On the Worker side `ether_collab::share::web::WebPeers` implements `PeerEndpoint` (and its
  links `PeerLink`) over that port, so the controller drives a web endpoint exactly like
  the native str0m one. `PeerLink::buffered()` = bytes posted and not yet consumed by the
  data channel plus its `bufferedAmount`.
- **Relay only** is `PeerEndpoint::open(.., relay_only)`: on the web it becomes
  `iceTransportPolicy: "relay"`. Natively there is no TURN client (§11), so the UI never sends
  `relay_only` to a native engine (`nativeEngine()` in `ui/src/features/share/settings.ts`),
  and a native endpoint asked for it fails the open with a clear reason.
- The contract's `ShareEvent::{PeerEndpoint, PeerSignal}` and `ShareCommand::PeerSignal` are
  **unused** (kept in the contract, append-only; the controller answers `PeerSignal` with
  `Unsupported`).
- The signaling WebSocket stays in the Worker (`WebSocket` exists there:
  `ether_collab::share::signal::web::WebSignal`). Its `Origin` is the page's origin, which
  the service checks (§3.4).
- Without an installed port (a Worker started some other way) every web pairing fails with
  "no share port"; signaling still works.
- A **web host works** (the hub is in the Worker) as long as its tab stays open. Background
  tabs keep Workers and WebRTC alive, but laptops sleeping or tab discarding end the session
  (§11).

### 6.3 Defaults: what a fresh install uses (`share-integration`)

No setting is needed for Share → link → Join to work:

| | Default | Where it comes from | Override |
|---|---|---|---|
| Sharing services | real ones: native `signal::native::WsSignal` (tungstenite + rustls, `wss://`) and `native::NativePeers` (str0m); web `WebSignal` + `WebPeers` | `EtherController::share_services()` falls back to `ether_collab::share::default_services()`, so `ether-native` (desktop and `ether-server`) and `ether-wasm` construct the controller without calling `set_share_services` | tests inject `share::fake` |
| Signaling service | `https://etherealws.pages.dev/signal` (`DEFAULT_SIGNAL_URL`) | the Pages Function of §3.1 (shipped in the web release since #241) | Settings > Advanced > Signaling server (`SetServers`); a joiner uses the link's `?s=` |
| ICE servers | what the service advertises in `HostWelcome`/`JoinWelcome`: `stun:stun.cloudflare.com:3478` (`STUN_URLS` in `services/signal/wrangler.toml`), plus TURN if configured | §2.4 | Settings > Advanced > ICE servers (`Collab::SetIceServers`) |
| Invite links | `https://etherealws.pages.dev/join/<room>#<key>` | `DEFAULT_INVITE_ORIGIN` | `SetServers.invite_origin` (not exposed in the UI) |

**"Can't reach the sharing service"** is not a health probe. The app never calls
`/v1/health` (that route is for operators and smoke tests). The message is shown when the
room socket itself fails:
- **host**: `ShareState::Hosting{signal: Offline{reason}}` (the popover's alert, the pill's
  "Not joinable" tooltip with `reason`). The socket retries with backoff (1 s → 30 s);
- **joiner**: `JoinFailure::Network` on the join screen. It means the signaling socket
  failed or the service refused the hello (`NotHost`, `Malformed`). Failures after the
  introduction (the data channel dropped, or a bad handshake frame) are
  `JoinFailure::Unreachable` ("Couldn't reach the host's computer").

What the socket needs from the deployment:
- `GET <signal>/v1/rooms/<room>/{host,join}` must answer the WebSocket upgrade (`101`). Before
  the Pages Function shipped, Pages answered `/signal/...` with the SPA's `index.html` (200,
  no upgrade), which every client reported as "can't reach the sharing service".
- **Origins** (`ALLOWED_ORIGINS`, §3.4): the desktop app's signaling socket is opened by the
  engine (Rust), which sends **no `Origin`** and is always allowed, so the Tauri origins
  (`tauri://localhost`, `http://tauri.localhost`) only matter for webview requests (the
  health route). The web app's Worker socket sends the page's origin: production and
  preview deployments (`https://*.etherealws.pages.dev`) and `http://localhost:*` are
  allowed. `http://127.0.0.1:*` is not (use `localhost` for dev against the real service,
  or a local service).

Known v1 behaviour: **Stop** while the service is unreachable cannot send `CloseRoom`, so
joiners see "<host> went offline" rather than "stopped sharing", and the room is forgotten
by its 30-day TTL.

## 7. Lifecycle

### 7.1 Host

1. **Share** (`Start`) on the open project:
   - If no `share.json`, create a `HostShare`: random room, host token, edit and listen keys.
   - Start the hub. The host's site joins it over the loopback, its `SnapshotData::sites`
     seeded from `share.json.sites` (so resends from members who come back are recognized
     across host restarts).
   - Open the host socket (`HostHello` with all doors). `State = Hosting{signal:
     Connecting}`, then `Online` with the two links.
   - Editing never waits for any of this.
2. **Joiners arrive**: `PeerArrived{peer}` → `PeerEndpoint::open(peer, offer: false)` → answer
   → data channel → handshake (§4.3) → `Accept` → the joiner's site joins the hub as a
   connection. Participants are updated, and the host sees the toast "Ada joined".
3. **Stop sharing**: `CloseRoom`, close every link, delete the secrets. Joiners get
   `SharingEnded` (their reconnect finds `InvalidInvite`), and their copies stay as offline
   copies.
4. **Quit, or open another project** (sharing paused, links stay valid): the hub stops and
   the sockets close. `share.json.resume = true`, and `sites` is saved with the project.
   Joiners see "Diego went offline". Opening the project again resumes sharing
   automatically (Settings toggle "Resume sharing when I open a shared project", default
   on; decision 13). Opening another project while people are connected first asks "Ada and
   Tom are in Song. Switch anyway?".
5. **Host restart and the log**: the hub's log is in memory, so a restarted hub is a new
   session epoch created from the host's copy. Joiners resync (COLLAB.md §4: an epoch
   mismatch means snapshot + log). Their unsequenced edits from this app run are resent and
   rebased as today.

### 7.2 Joiner

- **First join** (`OpenInvite`): `Contacting` (`JoinHello` with the link door) →
  `Connecting` (offer, ICE, DTLS, handshake with `Credential::Link`) → `Ready{invite}` (join
  screen) → the user clicks Join → `AcceptInvite`:
  - write `share.json` (`CopyShare` with the member key);
  - send `Accept`;
  - start the collab session over the link: `Syncing` (media, snapshot, log), then the
    project opens, `Joined{link: Online}`.
  - If the store already has this `ProjectId` (an old copy), today's adoption logic runs. A
    copy with offline work is kept as "Song (local copy)" with a `LocalCopyKept` notice.
- **Leave** (`Leave`): `CollabMessage::Leave`, close. The project stays open as the offline
  copy (saved, `share.json` kept), and Recents shows it with "Reconnect".
- **Reopen / reconnect** (opening an offline copy from Recents, or `Reconnect{project}`):
  the same flow with `Credential::Member` and no confirmation screen. The collab session
  sends `SyncRequest{epoch, index}`. If the hub still has that epoch, only the missing log is
  sent; otherwise the full snapshot. Offline edits made while disconnected **across an app
  restart** are reconciled with the existing backup rule (kept as a "(local copy)" if they
  diverge; decision 9). Edits made during a drop within one run stay pending and are merged
  on reconnect, as today.
- **Connection lost** (ICE consent failure, network change): the collab link closes and the
  controller's existing backoff (0.5 s → 10 s) reconnects through the share connector:
  signaling, ICE, member handshake. `HostLink::Connecting{attempt}` shows "Reconnecting…" in
  the pill. Pending edits stay pending and are resent (COLLAB.md §4).
- **Host goes offline mid-session**: the data channel closes, the reconnect finds
  `HostOffline` at the signaling service, and `Joined{link: HostOffline}` plus the notice
  "Diego went offline: you're on an offline copy" follow. The joiner keeps its signaling
  socket open (≤ 10 min, then retries with backoff). When the host returns, `JoinWelcome`
  arrives and the joiner reconnects at once ("Diego is back"). The project stays editable.
  Edits stay pending in memory and sync when the host is back (banner: "Your changes will
  sync when Diego is back. Keep Ethereal open").
- **Removed or sharing stopped**: the reconnect gets `InvalidInvite` → `SharingEnded`
  notice, the copy becomes "Sharing ended" in Recents (`ProjectShareInfo.active = false`),
  and "Make a private copy" (`Detach`) turns it into a normal project.

### 7.3 Sequence: first join

```
Ada (web)            Ada controller          signaling (DO room R)       Diego controller (host, hub)
   │ open /join/R#1k      │                          │                          │ HostHello{token, doors}
   │ landing: Continue ──►│                          │◄─────────────────────────│ (when Diego shared)
   │ OpenInvite{link} ───►│ door = HMAC(k, R)        │ HostWelcome{ice} ───────►│
   │                      │ wss /v1/rooms/R/join ───►│                          │
   │                      │ JoinHello{door} ────────►│ SHA256(door) ∈ doors ✓   │
   │                      │◄──── JoinWelcome{peer 7} │ PeerArrived{7} ─────────►│ endpoint.open(7, answer)
   │◄ PeerEndpoint Open   │                          │                          │
   │  (offer, DC "ethereal-collab/1")                │                          │
   │ PeerSignal{Offer} ──►│ Signal{Offer} ──────────►│ Signal{7, Offer} ───────►│ answer
   │◄ PeerSignal{Answer} ◄│◄──────── Signal{Answer} ◄│◄──────── Signal{Answer} ─│
   │ ⇄ trickle ICE (same path, both ways)            │                          │
   │ ═════════════ ICE + DTLS: data channel opens (P2P, or via TURN) ═══════════│
   │ share port: {7, open, fps}                      │ (Ada closes her socket)  │
   │                      │ Hello{Link, proof(k, fps), "Ada"} ═════════════════►│ try K_edit/K_listen ✓ Edit
   │                      │◄═════ Welcome{host_proof, Edit, member m, K_m, "Diego", "Song"}
   │                      │ verify host_proof ✓      │                          │
   │◄ State Joining{Ready{Diego, Song, Edit}}        │                          │
   │ "Diego invites you to Song: Join?" → Join       │                          │
   │ AcceptInvite ───────►│ write share.json (m, K_m)│                          │
   │                      │ Accept ════════════════════════════════════════════►│ store member m; SetDoors
   │                      │ CollabMessage::Hello + SyncRequest ════════════════►│ hub (Relay): media, snapshot,
   │                      │◄═══════════════════════ Media…, Snapshot, log, presence │ log, peers' presence
   │◄ ProjectLoaded, State Joined{Online}; toast on Diego's side "Ada joined"  │
   │ edits: Transaction ═════════════════════════════════════════════════════►│ sequenced, fanned out (echo)
```

### 7.4 Optional: hand over host (not in the first wave)

Designed so it can be added later as a single node (`share-handover`) without contract
breaks. The host picks an online **Edit** member and chooses "Make Ada the host":

1. The hub stops accepting transactions and flushes the log.
2. Over the encrypted channel it sends Ada the `HostShare` (room, host token, link keys,
   members, sites) in a new `PeerHandshake`-level frame. That frame is a new append-only
   variant.
3. Ada's app starts a hub from her fully synced copy and claims the room with the same
   token.
4. The old host becomes a member and everyone reconnects through the usual member path.

This is clearly optional (decision 11): it changes who holds the master copy, so it needs
the owner's go.

## 8. UX spec

All built from kit components (`Button`, `IconButton`, `Popover`, `Dialog`, `Menu`,
`Badge`, `Tabs`, `Toggle`, `TextInput`, `Select`, `Tooltip`, `Toast`/`ToastStack`) and
tokens. Peer colours are data, like track colours. The owner's UX passes on dev are
authoritative.

### 8.1 Top bar: Share button and session pill (`share-ui`)

- The `data-slot="collab"` slot renders `ShareControl`:
  - **Not shared**: `Button` "Share" (icon `Share2`, variant primary, size sm). Clicking it
    sends `Start` and opens the `SharePopover` anchored to it.
  - **Shared or joined**: the **session pill** (`SessionPill`): an avatar stack (≤ 3
    `PeerAvatar`s: initials in the peer colour, then "+N") followed by a status dot and
    label:
    - "Live" (green dot): hosting and signal online, or joined and online;
    - "Connecting…" (amber): signal connecting, `Joining`, or `HostLink::Connecting`;
    - "Diego offline" (grey): `HostOffline`;
    - "Not joinable" (amber, tooltip with the reason): signal offline while hosting.
    The existing per-peer chips (follow on click, listen menu) move into the popover's
    participant list, so the top bar has exactly one session element.
  - The relay-era `PresenceBar` join dialog goes to Settings > Advanced (§8.6). Its peer
    chips, follow, listen, the "Hide users and notes" toggle and `ChatToasts` stay mounted
    from `ShareControl`.

### 8.2 Share popover (host)

`Popover` titled "Share “Song”":
1. **Link row**: `Tabs` "Can edit" | "Can listen" (which link to show), then a read-only
   `TextInput` with the link and a "Copy" `Button` (toast "Link copied"). While the room is
   (re)opening there is a spinner instead ("Getting a link…"). If the signal is offline:
   "Can't reach the sharing service. People already here stay connected."
2. **You appear as**: name `TextInput` + 8 colour swatches (`IconButton`s with
   `aria-pressed`). Shown expanded the first time (no identity yet), collapsed after.
3. **People**: one row per participant:
   - `PeerAvatar`, name, a role `Select` ("Can edit" / "Can listen") for members, "Host"
     `Badge` for the host, an online dot, and "(you)".
   - The row `Menu` holds: Follow, Listen on <name> (COLLAB.md §9), "Remove from project".
   - Offline members are listed dimmed, with "last seen".
4. **Footer**: `Menu` "…" with "Reset edit link" / "Reset listen link" (confirm: "The current
   link stops working. People already in keep access."), and a "Stop sharing" `Button`
   (danger tone; confirm `Dialog`: "Stop sharing Song? Ada and Tom keep an offline copy.
   Links stop working.").

Joiner popover (opened from the pill): host first, participants, their role, "Leave"
(`Button`). Listen joiners also get "Listening to Diego" with "Stop listening" / "Listen
again".

### 8.3 Join screen (`join-flow`, `JoinScreen`, full-app `Dialog`)

| `ShareState::Joining` stage | Content |
|---|---|
| `Contacting` / `Connecting` | spinner, "Connecting to the invite…" (≤ ~2 s typically), Cancel |
| `Ready{invite}` | host avatar (big), "**Diego** invites you to **Song**", "You can edit" or "You can listen", avatars of `online`. A name field "Join as" if no identity yet. Note if `local_copy`: "You have an older copy of Song. It will be updated, and any offline changes are kept separately." Buttons: **Join** (primary, autofocus) / "Not now" |
| `Syncing` | progress bar (`received_bytes / total_bytes`), "Downloading Song…" |
| `HostOffline{local_copy}` | "Diego's Ethereal is closed. Song opens as soon as they're back." Spinner "Waiting…", plus "Open my offline copy" if `local_copy`, and Cancel |
| `Failed{reason}` | the message ("This invite link was reset or is no longer valid. Ask Diego for a new one.", "Couldn't reach Diego's computer (network). Try again, or ask Diego to enable a relay server.", "Update Ethereal to join this project.", "Diego's identity could not be verified.") + "Close" |

On `Joined`: the dialog closes and the project is open. Toast "You're in Song with Diego".

### 8.4 View only, offline banners

- Listen joiners: a "View only" `Badge` next to the pill. Editing gestures do nothing, and
  commands are refused (the UI can check `ShareState.Joined.role`).
- Joined with `HostOffline`: a thin banner under the top bar: "Diego is offline. You're
  working on an offline copy. Changes sync when they're back; keep Ethereal open."

### 8.5 Recents (`recents-shared`, `ProjectScreen`)

- Each project row with `ProjectSummary.share`:
  - a `Badge` "Shared" (role `Host`) or "From Diego" (copy), or "Sharing ended"
    (`active = false`);
  - an avatar stack of `participants` (≤ 3 + "+N");
  - "Live" when it is the open, currently connected project.
- Row `Menu` entries: host gets "Copy invite link" and "Stop sharing". A copy gets
  "Reconnect" and "Make a private copy" (`Detach`). Opening a copy reconnects
  automatically (§7.2).
- Duplicating or "Save as" never copies `share.json` (a duplicate is private).

### 8.6 Settings

`AudioSettingsDialog` becomes a `SettingsDialog` with `Tabs`:
- **Audio** (unchanged).
- **Sharing**: identity (name, colour), "Resume sharing when I open a shared project"
  (`Toggle`), "Automatically listen to the host when joining with a listen link"
  (`Toggle`, on).
- **Advanced**:
  - Signaling server (`TextInput`, empty = default);
  - ICE servers (STUN/TURN list; the existing `SetIceServers`) and "Hide my IP (relay
    only)" (`Toggle`);
  - **Remote engine** (the `ConnectDialog` content moves here);
  - **Relay session** (the legacy server/session/token join form from `PresenceBar`).

The identity lives in UI settings (`localStorage`). It is sent with `SetIdentity` at startup
and on change.

### 8.7 Toasts (`ShareEvent::Notice` → kit `Toast`)

"Ada joined" (avatar colour) · "Ada left" · "Diego went offline: you're on an offline copy"
· "Diego is back" · "Diego stopped sharing Song. Your copy stays on this computer." ·
"Offline changes kept as “Song (local copy)”" · "Link copied" · "Link reset: the old one no
longer works". At most 3 on screen (the `ChatToasts` stack rules).

## 9. Migration: keep the relay as an advanced option (recommended)

- **Keep** `ether-collab-relay` and `CollabCommand::Join` as is, behind Settings > Advanced
  > "Relay session". Cost: none. The hub reuses the same `Relay` code, so the relay stays
  tested by the same tests. It serves always-on setups: a studio server where the "host" never
  sleeps (relay + `ether-server`), or LANs without internet access.
- The top-bar "Collab" join dialog disappears (the Share button replaces it). The presence,
  listen and chat UI keep working in both modes, because they read `CollabEvent`s.
- **Retiring** the relay would save little: the binary and its tests, but not the `Relay` state
  machine, which the hub reuses. Revisit once accounts exist (decision 10).

## 10. Security and privacy

- **What the signaling service sees**: room ids, door hashes and doors, the host token hash,
  client IPs (in sockets and in ICE candidates inside SDP), SDPs (codecs, DTLS fingerprints),
  and connection times. It **never** sees link or member keys, project names, participant
  names, ops, media or chat.
- **End-to-end encryption**: everything after the introduction travels in the DTLS data
  channel, peer to peer or through TURN (TURN sees ciphertext only). Fingerprint
  substitution by the signaling service is defeated by the fingerprint-bound proofs (§4.2).
  Listen audio is DTLS-SRTP, as today.
- **Capability leakage**: a link is a bearer capability. Anyone holding it can join until it
  is reset, so the popover says "Anyone with this link can edit". Mitigations:
  - the key stays in the fragment (never in server logs), and the web app strips it from the
    address bar after use;
  - listen links are separate from edit links;
  - members get their own keys, so resetting a link is cheap and does not kick anyone;
  - "Remove" revokes one member.
  `LinkKey`'s `Debug` never prints the secret. `share.json` is never replicated, exported
  or copied.
- **Host hardening**: every remote op still goes through `Project::apply` and resolve (COLLAB.md
  §7). The hub enforces roles (§2.3), identity (site and name pinned per connection), the
  relay's limits (sites, rate, log, media caches, queue budgets) and the per-connection
  throttles of `relay::limits`. A joiner can only reach the host after passing a door, and can
  only stay after a valid proof. Failed proofs are counted per pairing, and three in a row
  make the host `EndPeer` that pairing.
- **IP addresses**: as with any P2P app, peers learn each other's IPs from ICE candidates
  (browsers use mDNS for host candidates). "Hide my IP (relay only)" uses TURN only.
- **Rate limits**: §3.4 (service), the relay limits (hub), and handshake timeouts (10 s from
  data channel open to `Hello`, 60 s to `Accept`).

## 11. Platform matrix and known limitations

| | Host | Join (edit) | Join (listen: hear the host) |
|---|---|---|---|
| **Desktop** (macOS, Windows, Linux) | yes: hub in the engine process, str0m data channels | yes (str0m) | yes. Audio needs the webview's WebRTC receiver (COLLAB.md §9.1). On WebKitGTK builds without WebRTC, listen-audio is unavailable (the read-only replica still works) |
| **Web** (Chromium, Firefox, Safari) | yes, while the tab is open: hub in the controller Worker, UI `RTCPeerConnection`s via the share port | yes | yes (browser receiver) |
| **`ether-server`** (headless) | yes (always-on host; owner decision on exposure) | n/a | n/a |

Known limitations:
- **Symmetric NAT on both sides without TURN**: ICE fails (`JoinFailure::Unreachable`, "ask
  the host to enable a relay server"). STUN covers most home and office NATs. Corporate
  networks and some mobile carriers need TURN (§3.4). **Native has no TURN client** (str0m):
  a native peer connects through TURN only when the *other* side allocates a relay
  candidate, so native↔native across two symmetric NATs fails even with TURN until a native
  TURN client exists (future: `turn` crate client in the share thread).
- The host must be online for anyone to join (owner decision). Hand-over is §7.4.
- A restarted host starts a new epoch: joiners resync fully once (snapshot + media cache).
  This is fine for typical projects, but a large media library means a large first sync.
- Web hosts end when the tab is closed, discarded, or the laptop sleeps.
- Offline edits across an app restart are forked as a "(local copy)", not merged (decision 9).
- ≤ 16 people online, ≤ 64 members per shared project.
- `ethereal://` registration needs an installed (bundled) app. Dev builds don't register it.

## 12. Implementation plan

Waves. Every node depends on base-115 (this PR). Acyclic: the first five nodes run in
parallel; `share-integration` comes last. Paths are in `.github/ownership.toml`, and acceptance
details are in ROADMAP.md "Sharing (base-115)".

| Node | Owns (summary) | Deps | Acceptance (summary) |
|---|---|---|---|
| `signal-service` | `services/signal/**`, the Pages Function proxy `scripts/release/web/functions/**` | base-115 | `RoomCore` implements §3.3 with unit tests (claim, wrong token, doors, offline wait + late host, signal routing, EndPeer, CloseRoom, TTL alarm, every limit). `wrangler dev` smoke script. A Node `ws` adapter for tests (`test/server.ts`), used by e2e. No deploy |
| `p2p-transport` | `crates/ether-collab/src/share/{native,web,signal}/**`, the share port in `ether-wasm` + `apps/web/src/engine`, `ui/src/features/share/endpoint/**` | base-115 | `SignalLink` native (tungstenite + rustls) and wasm (Worker `WebSocket`). `PeerEndpoint` native (str0m data channel, host + srflx candidates) and web (UI `RTCPeerConnection` over the share port). Fingerprints in `Connected`. Test: two native endpoints connect over loopback through an in-memory signal and exchange 20 MiB of fragmented frames in order. Web: vitest with a fake RTC; Playwright two-context data channel echo |
| `share-engine` | `crates/ether-controller/src/share/**`, `crates/ether-collab/src/share/{hub,handshake,keys,fake}.rs`, tests `share*.rs`, mock `share.ts`; shared touches in `collab/mod.rs` (connector per session, `left_sites` seed), `relay/mod.rs` (`set_snapshot_source`, `set_color`), `handlers.rs` (view-only refusal) | base-115 | Every `ShareCommand` (removes its `share_prewire` assertions). Hub with roles (§2.3). Handshake and key derivations with **frozen test vectors** (§4.2-4.3). `share.json` lifecycle (no copy on SaveAs/Duplicate). Integration over fake signal + in-memory `PeerEndpoint`: host + 2 joiners converge (reuse the collab property test with the hub), listen role read-only + chat, link reset keeps members, remove, stop, host restart (new epoch, resend), joiner reconnect, offline copy + "(local copy)" |
| `share-ui` | `ui/src/features/share/{ShareControl,SharePopover,SessionPill,…}`, the top-bar slot in `App.tsx`, `ui/src/features/collab/{PresenceBar,index,store}` (move the chips, relay dialog out), Settings dialog with tabs (`audio-settings/**`, `remote/ConnectDialog.tsx` move) | base-115 | §8.1, §8.2, §8.4, §8.6, §8.7 against `MockShare` (`simulateJoin`, `simulateHostOnline`), RTL tests, screenshots light/dark. Remote engine in Settings > Advanced |
| `join-flow` | `apps/desktop/src-tauri/**` (deep-link and single-instance plugins, scheme), `apps/web/src/main.tsx` + `apps/web/src/join/**` (landing), `ui/src/features/share/join/**` (`JoinScreen`, deep-link listener), web `_redirects` | base-115 | §5 and §8.3 against `MockShare`. Landing works without booting wasm. The key is stripped from the URL. A cold-start and warm deep link reach `OpenInvite` (Tauri test or a documented manual owner/laptop check). Playwright: `/join/...` → landing → Continue → Ready → Join (mock) |
| `recents-shared` | `crates/ether-{native,wasm}/src/store.rs` + `ether-controller/src/memory.rs` (read `share.json` into `ProjectSummary.share`), `ui/src/features/project/**` | base-115 | Badges, avatars, menus of §8.5 against stores with fixture `share.json`s. Duplicate/SaveAs never copy `share.json` (native + wasm store tests) |
| `share-integration` | `apps/web/e2e/share*.spec.ts`, `docs/SHARING.md`, `docs/COLLAB.md` cross-refs, `ShareServices` wiring in `ether-native`/`ether-wasm` defaults | all of the above | Two browser contexts through the Node signal adapter: share → copy link → open in the other context → Join → edits both ways, chat, host leaves → offline copy → host back → reconnect. Native↔web join on the devbox (loopback). The owner's laptop checks (desktop deep link, macOS) listed in the PR |

Later (optional): `share-handover` (§7.4), `native-turn-client` (§11), `offline-merge`
(persist pending ops across restarts; decision 9).

### 12.1 Tests across the nodes

| Test | What it covers |
|---|---|
| `apps/web/e2e/share.spec.ts` | Two browser contexts (each with its own wasm engine) through the Node signaling adapter: share → Copy → open the link in the other browser → landing → Continue in browser → "Diego invites you to …" → Join; edits both ways; chat both ways; the host closes its tab → "Diego is offline" banner, the joiner edits its offline copy; the host opens Ethereal again and reopens the project → sharing resumes, the joiner reconnects by itself ("Diego is back") and its offline edit syncs; Stop sharing → "stopped sharing", the copy stays. The joiner's browser takes the real launch path (no WebDriver flag), and no project screen covers the join |
| `apps/web/e2e/share-native.spec.ts` | Native ↔ web on loopback: `ether-server` (the desktop engine, headless, null audio, real `default_services()`: tungstenite + str0m) hosts and a browser joins, then a browser hosts and `ether-server` joins (`OpenInvite` → `Ready` → `AcceptInvite`); edits both ways, Stop, Leave |
| `apps/web/e2e/p2p.spec.ts` (`p2p-transport`) | Data-channel echo browser ↔ browser and browser ↔ native str0m, without the engine |
| `apps/web/e2e/join.spec.ts` (`join-flow`) | Landing, key stripping, pasted links, failures (against `MockShare`) |
| `crates/ether-controller/tests/share.rs` (`share-engine`) | Every command over the fakes, plus two found by the e2e: a joiner leaving signaling before the host sees its channel (`FakeNet::set_answerer_lag`), and Stop while the service is down |
| `services/signal` tests | `RoomCore`, limits, the Node adapter, the Pages Function |

Running them: `pnpm --filter @ethereal/web test:e2e e2e/share.spec.ts e2e/share-native.spec.ts`
(builds the wasm; the native spec also builds `ether-server`). The adapter is started as
`node --experimental-transform-types services/signal/test/server.ts --port 0` (Node's default
type stripping rejects its parameter properties).

Not covered on the devbox (owner's laptop checks, listed in the PR): the desktop deep link
(`ethereal://`, cold and warm start, macOS), and a real share between two machines through
the deployed service and STUN.

## 13. Decisions for the owner

Each choice has a recommendation. Everything else follows from the three final decisions.

1. **Link key in the URL fragment** (`/join/<room>#<key>`) rather than all in the path
   (`/join/<code>`). The key never reaches a server or its logs. The link looks the same to
   users. *Recommend: fragment.*
2. **Signaling URL**: mount the Worker's Durable Object behind a Pages Function at
   `https://etherealws.pages.dev/signal` (same origin, no CORS/COEP friction, one domain), or
   a standalone `ethereal-signal.<account>.workers.dev`. *Recommend: Pages Function.*
3. **Default STUN = `stun.cloudflare.com`**. COLLAB.md §10 avoided third-party STUN, but the
   signaling already runs on Cloudflare and P2P across NATs needs STUN. *Recommend: yes,
   overridable in Settings.*
4. **TURN**: off by default. Optional Cloudflare Realtime TURN through service secrets (paid
   beyond the free tier) when the owner wants "always connects". *Recommend: off now, revisit
   after real-world failure rates.*
5. **Native WebRTC = str0m** in the engine process (already a dependency), with the web
   using the UI's `RTCPeerConnection` over a `MessagePort`. *Recommend: as designed (§6).*
6. **Native TLS** for the signaling socket via `tungstenite` + rustls (≈ +1 MB). It also
   enables `wss://` relays natively. *Recommend: yes.*
7. **Member keys survive a link reset.** "Reset link" stops new joins with the old link
   without kicking anyone. "Remove" and "Stop sharing" revoke. *Recommend: yes.*
8. **Listen links**: a read-only replica, auto-listen to the host's audio, chat allowed.
   *Recommend: yes.*
9. **Offline edits across an app restart** are kept as a "(local copy)" fork (today's logic),
   not merged. Within one run, edits during a drop merge on reconnect. *Recommend: v1 as
   designed. Persisting the pending queue (`offline-merge`) is a later node.*
10. **Relay**: keep it as Settings > Advanced > Relay session. *Recommend: keep (§9).*
11. **Hand over host**: designed (§7.4) but not scheduled. *Recommend: defer.*
12. **Limits**: 16 online, 64 members per project, 30-day room TTL. *Recommend: as is.*
13. **Auto-resume sharing** when the host reopens a shared project. *Recommend: on, with the
    Settings toggle.*
14. **The joiner offers, the host answers** (the host stays passive). *Recommend: yes.*
15. **The project name is revealed only after authentication** over the encrypted channel.
    The join screen shows "Connecting…" for ~1-2 s before "Diego invites you to Song".
    *Recommend: yes (private). The alternative is putting the name in the room record.*
16. **Identity** in UI settings (`localStorage`), pushed with `SetIdentity`. The colour is a
    preference and the host resolves clashes. *Recommend: yes.*
17. **`ether-server` as a host** (always-on studio box). *Recommend: allowed (it comes for
    free), not advertised until accounts exist.*
