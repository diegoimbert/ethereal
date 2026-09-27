# Real-time collaboration (`collab` node)

Status: approved design (manager, with the clarifications folded in below). Contracts it relies on:
CONTRACTS.md §11.6 (`SiteId`, `ActorId`, `OpOrigin`, `StampedTransaction`,
`CollabCommand`/`CollabEvent`/`CollabMessage`, `Patch::origin`) and §11.5 (remote engine).

## 1. Topology: replicated documents, one relay

```
 UI ─┐                         ┌─ UI (browser, maybe on another machine)
     │ (Tauri / wasm / ws)     │ (remote engine, ws)
 site A: controller+engine     site B: ether-server (controller+engine)
     │  CollabMessage (ws)     │
     └────────► relay ◄────────┘   ◄── site C (web: wasm controller in a Worker)
             (sequencer)
```

- Every participant is a **site**: one controller with its own engine, audio device, project
  store and media. Each site holds a full replica of the document. A site's UI can be local
  (desktop, web) or remote (a browser on `ether-server`, remote-engine topology), so "UI on
  another machine, projects engine-side" still holds.
- Sites connect to a **relay** (`ether-collab-relay`, dev port base `+3`,
  `PORT_OFFSETS.collab`) over WebSocket. The relay has no document model: it authenticates
  sites, **sequences** transactions (one total order per session), fans messages out, keeps
  the log (+ latest snapshot and media blobs) for late joiners and reconnects, and assigns
  presence colors.
- Rejected alternative: everyone as clients of one `ether-server`. That already works
  (remote-engine) and stays available, but it means one audio engine: only the host hears
  the project, the others need a separate audio path. Replicas give everyone local,
  latency-free audio and local undo, and survive a peer leaving.
- **Transport (play/stop/locate/record/arm) is per-site** and never shared, except while a
  site listens on a peer (§9: its transport follows the host). What else is local is
  listed in §2.1.

## 2. Consistency: server-sequenced op log (no CRDT library)

Our ops are already CRDT-friendly: entities by ULID, single-field `Update`s (LWW registers),
whole-entity `Insert`/`Remove`, fractional `OrderKey`s, exact inverses. A library (yrs,
automerge, loro) would force a second document representation and still not know our
invariants (routing cycles, "clip on an existing track of the right kind", tempo point at
0, ...). So: **a total order from the relay + deterministic, validating application**.

- A local edit commits exactly as today (optimistic, zero latency): `History::commit`, patch
  to the UI, engine effects. It is then stamped `OpOrigin { site, actor, seq }` (`seq` =
  1, 2, ... per site) and sent as `CollabMessage::Transaction`. It stays **pending** until
  the relay echoes it back.
- The relay appends every `Transaction` to the session log (global index = log position) and
  broadcasts it to every site, **including its author** (the echo is the ack). One TCP
  stream per site ⇒ every site sees the log in the same order.
- Each site keeps `confirmed` = log applied in order (implicitly: `live` minus pending), and
  `live = confirmed ⊕ pending` (the doc the UI and engine see). On a remote transaction:
  1. undo the pending transactions (their recorded inverses, newest first) ⇒ `confirmed`;
  2. apply the remote transaction with **resolve** (below) ⇒ new `confirmed`;
  3. re-apply the pending transactions (the original ops, as sent) with **resolve**, and
     record their new inverses.
  One patch (`Patch::origin = Some(remote origin)`) covers every entity touched by 1-3.
  Without pending transactions (the common case) this is just step 2.
- **The echo of an own transaction is processed like any sequenced transaction**: it is the
  head of `pending` (the relay keeps each site's FIFO order), and "undo pending, apply it with
  resolve on `confirmed`, re-apply the rest" gives exactly the state we already have, because
  the live doc is maintained as `fold(resolve, confirmed, pending)` at every step (pending
  ops are re-resolved against the latest `confirmed` on every rebase). So the implementation
  pops it (and keeps its current inverse). In debug builds every own echo is also processed
  the long way on a copy (undo pending, resolve it on `confirmed`, re-apply the rest) and
  asserted equal to the live document (`check_echo`); every collab test runs with it.
  An own-site transaction is an echo **only if it matches the head of `pending`**; any other
  (ours from before a leave + re-join on the same controller, which the snapshot + log
  replay) is applied exactly like a peer's. `seq` is per controller lifetime (it survives
  Leave/Join) and is raised to the highest own `seq` seen in the log, so a re-joined site
  never reuses a sequenced `seq` (the relay would drop it as a resend).
- **Convergence invariant** (checked by the property test): on every site, after the log is
  fully delivered, `confirmed` = `fold(resolve, join snapshot, relay log)` and `pending` is
  empty, so `live` is identical everywhere (up to the site-local fields of §2.1: the mix,
  loop and metronome settings). The
  property test (random concurrent edits including track/group/clip/device deletes that
  cascade, concurrent undo/redo, random delivery interleavings) checks all replicas are
  equal and valid, and that a fresh site replaying only the relay's snapshot + log (no
  optimistic state) reaches the same document. Every
  derived op (delete-wins cascade, detached references) is computed by resolve from the
  state it is applied to, which for a sequenced transaction is the confirmed prefix: never
  from another site's optimistic state.
- **resolve(op)** is a pure function of (state, op), identical on every site, applied op by
  op (not all-or-nothing, so one conflicting op doesn't drop a whole batch). Every op still
  goes through `Project::apply`, so **every intermediate state validates**:
  - `Update` of a missing entity, `Remove` of a missing entity, `Insert` of an existing id:
    skipped (delete wins; ids are ULIDs so an existing id means a replay).
  - `Update`/`Insert` rejected by validation (dangling reference, e.g. a clip on a track
    deleted concurrently; a kind mismatch; a routing/parent/sidechain cycle created by two
    concurrent edits; invalid values): skipped.
  - `Remove` rejected with `HasChildren` (a peer added a clip/device/send/lane/... to an
    entity being deleted): **delete wins**: the controller's own cascade (`DocCtx::delete_*`,
    with a host that returns no plugin state, so it is deterministic) removes the dependents
    and detaches references (outputs → Default, sidechains → None, ...), then the remove.
  - `Insert` or `Order` update whose `OrderKey` equals a sibling's (two sites inserted at
    the same spot): applied as is. Ties already sort by id everywhere (`by_order`), so all
    sites show the same order.
  Convergence: `confirmed` on every site = resolve-fold of the same log from the same
  snapshot; `live` differs only by that site's own pending ops, which every other site
  applies (with the same resolve) once sequenced.
- Everything else that is not an op stays site-local and derived: media `frames` fix-ups
  (background decode), plugin param mirroring after load, peaks, caches. They never
  produce a transaction.

### 2.1 Shared and site-local state

| Shared (ops are sent) | Site-local (never sent, remote values ignored) |
|---|---|
| every entity table (tracks, clips, notes, devices + params, sends, lanes/points, tempo, signatures, warp markers, media, markers, MIDI mappings, drum pads, chat messages, pinned notes: §12) | transport: play/stop/locate/record, playhead, count-in |
| track volume, pan, routing, names, colors, order; drum pad volume, pan, choke group | **mute and solo, per user**: track mute and solo (`TrackChange::Mute`/`Solo`, group tracks included), drum pad mute (`DrumPadChange::Mute`), drum pad solo (`SetPadSolo`, runtime already); record-arm (runtime already) |
| settings: project name, swing, swing grid | settings: loop enabled + loop region, metronome on/off, volume, accent, sound, count-in bars |
| | MIDI learn mode/gestures, selection (shared only as presence), undo history |
| | the live recording view (`RecordingEvent::Progress`, live chunks/notes): events, never ops; only the committed take (media pushed first, then its `Insert`s) replicates |
| | missing-plugin bypass (runtime engine state, never an op) |
| | UI preferences such as "hide others" (§12.4): never a command |

Local-only ops are applied and undone locally as usual but filtered out of the stamped
transaction (a transaction with only local ops is not sent; a mixed one, e.g. a rename and
a mute in one step or its undo, sends only its shared ops). Local undo/redo of a mute or
solo never goes out. A receiver drops local-only ops anyway (older peers sent mute).

**Mute and solo are per user** (`resolve::LocalMix`): each site mutes and solos for itself,
the way each one has its own headphones.
- Sent `Insert`s of tracks and drum pads carry no mute/solo (`resolve::outgoing`), and
  session snapshots carry none (`collab_ether`), so nobody's mix is ever on the wire.
- Applying a peer's transaction keeps this site's mix: it is read before the rebase and put
  back after it (re-applied pending inserts, and legacy peers' inserts carrying a mix, would
  otherwise change it). Tracks and pads created by others start unmuted and unsoloed; tracks
  deleted remotely just disappear.
- Join, re-join and snapshot adoption keep this site's mix: it is taken from its open copy of
  the project, else from its stored copy, overlaid on the snapshot, and kept as a seed for
  the log replay (tracks the log re-inserts that this site had get their mix back).
- The replicated document converges with the mix masked out (`shared_part`, the property
  test's `shared()`; the property test toggles random mutes/solos, and a fresh replaying
  site ends with no mix at all). A stored copy differing only by its mix is not offline
  work (no "(local copy)").
- Persistence: each site saves its own mix in its own `.ether` file (a normal save).
  Outside a session nothing changes: mute and solo are ordinary undoable edits.
- **Listen on a peer** (§9) plays the host's mix, the host's mutes and solos included: the
  listener hears the host's engine output, not its own mix.

A joiner also keeps its own local settings over the snapshot's.

### 2.2 Writes that are not user edits

- Plugin GUI param edits (`ParamEdited`) already become undoable `Set Param` edits: they
  replicate like any edit.
- Save-time plugin state capture (opaque, non-parameter state, e.g. a preset loaded in the
  plugin GUI) replicates **at most once per change**: before a save (explicit or autosave)
  in a session, `collab_before_save` reads each instantiated plugin's live state and, only
  if it differs from this site's baseline for that device (the last state it sent, received
  or started from), applies one `DeviceChange::Plugin` update to the document and sends it
  as a stamped transaction **outside the undo history** (label "Plugin State"). Saving again
  with unchanged states sends nothing. The saved file itself is still written from a copy
  (`serialize`). The session snapshot (creation and compaction) carries live states too.
- A peer receiving a replicated state re-creates its running instance from the document's
  state (`EngineState::request_reload_from_doc`, base-44; a normal re-create would keep the
  instance's live state) and takes that state as its baseline, so its next save never sends
  its old state back (no ping-pong).
- Missing plugins on a peer: the existing handling (the engine bypasses a device it cannot
  instantiate) applies. It is runtime state, never an op, so it never replicates; such a
  site has no live state, so its saves never overwrite the replicated state either (its
  document and file keep the others' state).
- Media length fix-ups from background decode and param mirroring after plugin load: local
  derived data (above).
- ID minting (`copy_rack_pads`, duplicates, `SliceCommand::Auto`) is not a hazard: we sync
  **ops**, which carry the ids the author minted; commands are never replayed on another
  site. Resolve never mints ids (cascades only remove/detach).

## 3. Per-site undo

Undo only ever reverts **own** transactions: remote transactions never enter the local
`History`. With peers editing, a naive inverse would clobber their work, so in a session
undo/redo go through `History::undo_with/redo_with` (`history.rs`) with
`collab::resolve::apply_guarded`:
- **Value guard.** Each inverse `Update`/`Settings` op is applied only if its field still
  holds the value the step wrote (the step's forward op, aligned with the inverse). If a
  peer (or anything else) changed the field since, the op is **skipped** and the later value
  wins. No per-field bookkeeping is needed: the document itself is the witness.
- Inverse `Insert`/`Remove` go through resolve: re-inserting into a parent a peer deleted is
  skipped; removing an entity a peer added children to cascades (delete wins).
- The ops actually applied become a new stamped transaction (sent like any edit; so an
  undo made before a concurrent peer write reaches it is an ordinary later write and wins
  by sequence order). Redo is the guarded inverse of what the undo applied.
- Outside a session `History::undo/redo` behave exactly as before.

## 4. Join, late join, reconnect, leave

- `CollabCommand::Join { server, session, token, name }`: the site generates a `SiteId` from
  host entropy (kept for the controller's lifetime, so a reconnect is the same site) and
  connects to `server` (`ws://host:port/`; the session name is the URL path
  `/<session>` percent-encoded). Handshake = remote-engine's `ClientHello`/`ServerHello`
  (token, protocol version), then `CollabMessage::Hello { site, actor, name,
  protocol_version }`. The relay refuses (disconnects) a `protocol_version` other than its
  `COLLAB_PROTOCOL_VERSION` (`ether-collab/src/wire.rs`; 2 since base-62, §12), so sites of
  different builds never share a session and drift apart on messages they can't decode.
- Relay → joiner (after `Hello` + `SyncRequest`):
  - empty session: `SyncRequest { site: joiner, version: [] }` = "you create it": the site
    uploads its media (`Media` chunks) and a `Snapshot` of its open project. Other joiners
    wait until the snapshot arrives.
  - existing session: the media blobs, the latest `Snapshot`, the log after it, the other
    peers' `Hello` and `Presence`. The joiner saves its current project if dirty, writes the
    session project into its own store under the **same `ProjectId`**, opens it (media is in
    its `media/` already), and applies the log. Its own undo history starts empty.
  - **Existing local copy**: if the store already has that `ProjectId` and its saved content
    differs from the snapshot, it may be unsynced local work (edits made offline after a
    previous session) or just an older state of the session. The stored file is kept in
    memory and compared (shared part: site-local fields and opaque plugin states aside)
    with the document after every sequenced transaction of the replay; if it ever matches,
    it was a state of the session and nothing is kept. Otherwise, at the first quiet tick
    (or before the first local edit), it is saved as a new project named
    "<name> (local copy)" (with the project's media) and a notification says so. Nothing
    is overwritten silently.
- `Snapshot.data` (opaque on the wire, defined by `ether-collab`): JSON
  `{ epoch, index, sites: { site: last_seq }, ether: "<.ether file JSON>" }`, base64. The
  `epoch` is chosen by the site that creates the session (random) and kept by compaction
  snapshots: it names this incarnation of the session. Its `sites` are everything the
  creator's document includes: the creator's own map when it re-creates a session it was
  in (or the map it kept when it left the session with this project), so a returning site
  doesn't resend an edit the snapshot already has (re-sequenced after a newer peer edit,
  it would silently revert it).
- Reconnect (socket dropped, relay restart is out of scope): status `Connecting`, exponential
  backoff; on reconnect `Hello` + `SyncRequest { version = (epoch, confirmed index) (2 × u64
  LE) }`; the relay replays the log after that index (or snapshot + log if it was compacted,
  or if the epoch differs: the session emptied and was created again from another replica,
  whose indexes mean something else), then the
  site re-sends its pending transactions with `seq` above the last own `seq` seen in the log.
  The relay drops a transaction whose `seq` is not above the last sequenced one of that site
  (duplicate after a lost ack).
- Log compaction: past `N` entries the relay sends `SyncRequest { site, version: [] }` to one
  site with no pending ops, which answers with a `Snapshot` of its confirmed state; the relay
  truncates the log before it.
- `Leave` (or disconnect): the relay broadcasts `Leave { site }`. The document stays open as a
  normal local project (dirty; saving it is the user's choice). Opening/creating/closing
  another project while in a session leaves the session.

- Rebase and derived data: re-applying a pending transaction re-inserts the entities it
  created as they were sent, so derived local data written since (media `frames` from
  background decode, mirrored plugin params) is carried over explicitly
  (`derived_of_pending`/`restore_derived`; tested).

## 5. Media

A site that imports audio (or records a take) **pushes** the file before the transaction
that references it: `Media { file, hash, offset, total, data }` chunks of 1 MiB, sent as
remote-engine binary frames (`BinaryKind::Bytes`, the JSON header carries `data: ""`). The
relay caches blobs per session (bounded: `max_media_bytes`, default 1 GiB per session and
`max_total_media_bytes`, 4 GiB across sessions) and sends
them to late joiners before the snapshot. Because the relay keeps FIFO order, every site has
the file before the `Insert Media` arrives, so media never shows as missing. Receivers
stage chunks with the `ProjectStore` upload staging methods (in memory where the store has
none, e.g. OPFS today), verify `content_hash` against the declared hash, check `file` is a
relative path under `media/`, and write it only if (a) the document's `MediaRef` for that
file, when it has a `hash`, has **that** hash (not only the one the sender declared) and
(b) no different file is already stored there: media files are named per media id, so an
existing one is never replaced (identical bytes are a no-op). Then `ProjectStore::write`. Missing files at snapshot time are simply not sent
(they show as missing, as they would locally).

## 6. Presence

`CollabCommand::SetPresence` (cursor, selection, view), throttled to 10 Hz per site. The
relay stamps `site` (no spoofing) and assigns each site a color from a fixed 8-entry palette
in join order (the first free one); it forwards `Presence`, `Hello` (name) and `Leave`. The
controller emits `CollabEvent::Presence { peers }` (others only) and
`CollabEvent::Session { status }`. UI: `PresenceBar` (top bar `data-slot="collab"`): a
join/leave button + dialog (server, session, token, name; same pattern as the remote
`ConnectDialog`), and one avatar chip per peer in its color with its name; the peer's
selected tracks/clips get an outline in that color (only through kit components and
tokens; peer colors are data, like track colors). Presence v2 (live pointers, activity,
follow mode, listening indicator) is §8; chat, pinned notes and peers' playheads are §12.

## 7. Security

- Each connection's outgoing queue (`relay::server::conn_queue`) holds at least a full
  catch-up (the cached media chunks, snapshot, `max_log` transactions, peers), so a late
  joiner is never disconnected by its own catch-up and can't loop on reconnects; a site
  that falls further behind than that is disconnected and resyncs. The queue also has a
  byte budget (encoded size; `RelayServerConfig::max_queued_bytes`, 256 MiB, raised to
  fit a catch-up): a slow reader past it is closed (`4005`) like an idle one.
- Relay limits (all in `RelayConfig`): sessions per relay, sites per session, media cache per
  session and in total across sessions (chunks beyond it are still forwarded live but not
  cached for late joiners), log length before compaction and a hard log cap (past it the
  relay asks another peer for a snapshot and refuses new transactions from a site until
  the log shrinks: that site's link is closed and it resyncs on reconnect), and a per-site
  message rate limit (a site sending more than the cap per second is disconnected, like a
  slow reader). Presence, pointers and site-to-site messages have their own throttles
  (extra ones dropped; §8.5).
- Relay handshake and limits copied from `ether-server`: shared token in `ClientHello`
  compared in constant time, token required unless bound to loopback, `Host`/`Origin`
  loopback check when there is no token, handshake deadline + 64 KiB pre-auth limit +
  bounded pending handshakes, idle ping/timeout, write timeout, max sites per session and
  max sessions. Post-auth message limit 16 MiB (snapshots).
- A `SiteId` is held by one connection per session. A `Hello` claiming a site another
  connection holds starts a contest: the relay pings the holder and holds the newcomer's
  messages. Any sign of life from the holder (pong or message) within
  `RelayConfig::site_probe_ms` (4 s) refuses the newcomer (disconnect); otherwise the
  holder is dropped as half-open and the newcomer proceeds. A newcomer never evicts a live
  holder outright: the token is relay-wide, so any token holder could.
- The relay rewrites/validates identity: `Transaction.origin.site`, `Presence.site` and
  `Leave.site` must be the sender's `Hello` site (else dropped); `Pointer.site` is stamped;
  site-to-site messages must have `from` = the sender's site (§9.7).
- Receivers treat every remote op as untrusted input: it only enters the document through
  `Project::apply` (full validation); media paths are validated and hashes verified.
- No new unauthenticated surface: the UI↔engine protocol only gains the reserved
  `CollabCommand`s; the relay is a separate, token-protected listener.

## 8. Presence v2 (base-53, node `presence-v2`)

State of the art is Figma's multiplayer: live pointers, selections in the peer's colour, a
hint of what each peer is doing, follow mode. Everything here is **ephemeral** (never an op,
never in the document, never logged by the relay) and everything is additive to §6.

### 8.1 Two channels

| Channel | Carries | Rate | Relay |
|---|---|---|---|
| `SetPresence` → `CollabMessage::Presence` → `CollabEvent::Presence { peers }` (existing) | `PresenceState`: selection, edit cursor, view, **viewport**, **activity**, **following**, **listening_to**, **can_host** | ≤ 10 Hz per site (controller throttle), flushed on the next tick | cached per peer (late joiners get the latest), drops > 20 Hz |
| `SetPointer` → `CollabMessage::Pointer` → `CollabEvent::Pointer { site, pointer }` (new) | `Option<ArrangerPointer>` | ≤ 30 Hz while moving (`POINTER_MAX_HZ`), nothing while still | not cached, drops > 40 Hz **except clears** |

The pointer is its own channel because it is the only high-rate field: sending the whole
`PresenceState` (selected notes can be thousands of ids) at 30 Hz, and re-emitting the whole
`peers` list to the UI at 30 Hz per peer, would be wasteful. `CollabEvent::Presence` is
emitted only when a non-pointer field changes.

### 8.2 `PresenceState` fields (all `#[serde(default)]`, omitted when unset)

- `cursor: Option<Beats>` keeps its v1 meaning: the **edit (insert) cursor**, where a paste
  or recording would start. It is not deprecated and it is not the mouse pointer.
- `viewport: Option<ArrangerViewport { start, end, top_track, top_offset }>`: the visible
  part of the arranger, for follow mode. Horizontal range in beats; vertically, the track row
  at the top edge and the fraction of it scrolled past (row heights are local, so a pixel
  offset would mean nothing on another screen). Published when the view settles (throttled
  like the rest of the presence, i.e. ≤ 10 Hz while scrolling).
- `activity: Option<Activity { kind, target }>`: set when a gesture starts, cleared when it
  ends. `ActivityKind` = Dragging, Resizing, Drawing, Adjusting, Renaming, Recording, Other;
  `ActivityTarget` = Clip, Track, Device, Param, Notes (of a clip), Lane, Selection. The UI
  renders "Diego · dragging" next to the peer's pointer and/or on the target.
- `following: Option<SiteId>`: the peer this user follows (shown on the leader's chip).
- `listening_to: Option<SiteId>` and `can_host: bool` are **controller-owned** (§9): the
  controller overwrites whatever the UI sends.

### 8.3 Live pointers (arranger only)

- **Song coordinates, not screen coordinates**: `ArrangerPointer { beats, track, y }`, with
  `y` = 0..1 inside that track's row (including its expanded lanes); `track: None` = over the
  ruler/header area or below the last track (then `y` = 0). Zoom, scroll, row heights and
  folds are local to each user, so every receiver maps the pointer through its own layout.
- The UI sends `SetPointer` on `pointermove` over the arranger (rAF-coalesced) and
  `SetPointer { pointer: None }` on `pointerleave`, on blur, and when the arranger unmounts.
  The controller throttles to 30 Hz, keeping the **latest** value (a throttled update is sent
  on the next tick, so the last position always arrives) and never throttles a clear.
- Receivers render at display rate and **interpolate**: keep the last two samples with their
  arrival times and draw at `now - 1 interval` (≈ 40 ms) with linear interpolation (or a
  critically damped spring), snapping on a track change. A peer whose `track` is unknown
  (deleted), folded or scrolled out of view: clamp to the visible edge with an arrow, or
  hide; a clear or the peer's `Leave` removes it (the controller emits `Pointer { None }` on
  `Leave`).
- Look: arrow + name label in the peer colour (colours are data, like track colours),
  through kit components/tokens; drawn in one overlay layer over the arrangement
  (pointer-events: none), a shared touch on the arrangement (the owner's UX is
  authoritative).

### 8.4 Selection outlines, activity, follow, listening

- Selection outlines: the existing `selected_*` fields in the peer colour (§6,
  `highlightCss`). Unchanged.
- Follow mode: clicking a peer chip follows it (sets `following`). The follower applies the
  leader's `viewport` on every presence update: horizontal range exactly (zoom so that
  `start..end` fills its width), vertical scroll so that `top_track` is at the top with
  `top_offset`. Any local scroll/zoom, Escape, or clicking the chip again stops following.
  Following never changes the transport.
- "Listening to" indicator: a peer with `listening_to = X` shows a headphones badge ("Ada is
  listening to Diego"); a peer with `can_host` offers "Listen on <name>'s computer" in its
  chip menu (§9).

### 8.5 Rate limits (relay, per connection; `relay::limits`)

| Message | Sender | Relay |
|---|---|---|
| `Presence` | ≤ 10 Hz | at most one per 50 ms (20 Hz); extra dropped |
| `Pointer` | ≤ 30 Hz | at most one per 25 ms (40 Hz); **clears always pass**; extra dropped |
| `Signal`, `Listen`, `Unlisten`, `TransportRequest`, `StreamClock` | host: ≤ 10 Hz clocks per listener | shared token bucket, 200/s, burst 200; extra dropped |
| everything | | the existing global cap (`max_messages_per_second`, 2000/s): above it the site is disconnected |

## 9. Listen on a peer (hosted listening; base-53, nodes `stream-host` and `stream-listen`)

Everyone keeps their own replica and their own engine (§1). "Listen on Diego's computer"
makes a site **hear Diego's engine** instead of its own, with a shared playhead: useful when
the listener lacks a plugin, has a weak machine, or the group wants to hear exactly one mix.

### 9.1 Architecture

```
 Host (Diego)                                                Listener (Ada)
 ┌──────────────────────────────┐                            ┌──────────────────────────────┐
 │ engine: tracks → master      │                            │ engine: timeline held STOPPED │
 │   + metronome / count-in     │                            │   (previews, pad audition,   │
 │   ─► stream tap ─┐           │                            │    live MIDI still sound)    │
 │   + preview voice ─► speakers│                            │                              │
 │                  │ rtrb ring │    WebRTC (DTLS-SRTP, Opus) │ UI: RTCPeerConnection         │
 │ sender: Opus → RTP ──────────┼───── P2P, or via TURN ─────►│   → <audio> / AudioContext   │
 │  native: str0m (desktop,     │                            │   getSynchronizationSources() │
 │   ether-server)              │                            │   → playhead (§9.4)           │
 │  web: browser WebRTC (UI)    │                            │                              │
 └───────────┬──────────────────┘                            └─────────────┬────────────────┘
             │  Signal / StreamClock (to: Ada)      Listen / Signal /       │
             │                                      TransportRequest (to: Diego)
             └────────────────────────► relay ◄────────────────────────────┘
                                  (routes by `to`, enforces `from`;
                                   STUN on its UDP port, optional TURN)
```

- **Topology: a mesh from the host to each listener** (one peer connection per listener),
  fine for ≤ 8 listeners (`MAX_LISTENERS = 8`; Opus stereo ≈ 128 kbit/s up per listener).
  Future work: an SFU (the relay forwarding one uplink to many listeners) for larger
  audiences.
- **What is streamed**: the engine's **stream tap**, taken after master, the metronome and
  the count-in, and **before the browser preview voice** (`ether-core/src/stream_tap.rs`,
  pre-wired in `engine.rs::render_sub`; test `ether-core/tests/stream_tap.rs`). Sample
  previews stay private to each site. Pad auditions (clicking a drum pad) render through the
  pad's track chain into master, so a host's pad audition IS heard by its listeners; keeping
  them private would need a separate audition bus (future work).
- **Receivers**: always the UI's `RTCPeerConnection` (web build, and the Tauri webview on
  desktop: simplest, and it gives the jitter buffer, PLC, drift handling and congestion
  control for free). Audio plays through the webview's default output device, not the
  engine's device (acceptable for listening; a native receiver would fix it). Where the
  webview has no WebRTC (WebKitGTK builds without it, i.e. some Linux desktops), `Listen` is
  **disabled with a reason** in the UI ("this build's webview has no WebRTC", detected with
  `typeof RTCPeerConnection`) and the controller replies `Unsupported`; **hosting still
  works** there, because the native sender does not use the webview. Future work: a native
  receiver (str0m + Opus decoder playing into the engine output).
- **Senders**:
  - **Desktop / ether-server** (`StreamEndpoint::Engine`): `ether-native`. The audio thread
    only writes the tap (`EngineHandle::set_stream_tap`, RT-safe, pre-allocated `rtrb`
    rings, no allocation); a **sender thread** reads the ring, resamples to 48 kHz if needed
    (the workspace `rubato`), encodes 20 ms Opus frames (libopus), and drives one `str0m`
    `Rtc` per listener on one UDP socket (sans-IO: the thread owns the socket and the
    clock). Controlled through the defaulted `EngineBridge` hooks
    (`stream_capabilities`, `start/stop_stream_capture`, `stream_open/signal/close`,
    `poll_stream`; `ether-controller/src/streaming.rs`).
  - **Web** (`StreamEndpoint::Ui`): `RTCPeerConnection` is not available in Workers, so the
    sender runs in the **UI main thread**: the engine worklet gets a second output (the tap:
    master + metronome, minus preview) feeding a `MediaStreamAudioDestinationNode`, whose
    track is added to one `RTCPeerConnection` per listener. The UI declares it with
    `SetHosting { ui_sender: true }`; the controller lists the listeners in
    `ListenStatus.listeners` (endpoint `Ui`) and routes their signals to the UI.
- Opus settings (both senders): 48 kHz stereo, 20 ms frames, music mode, 128 kbit/s target
  (adapted by congestion control, 48-192), in-band FEC on, DTX off. Web: SDP
  `stereo=1; sprop-stereo=1; maxaveragebitrate=128000; useinbandfec=1`, track
  `contentHint = "music"`.

### 9.2 Message flows

Listen, happy path (host native; the web host differs only in who answers `stream_*`):

```
Ada UI        Ada controller        relay            Diego controller      Diego bridge
  │ Listen{host:D}  │                  │                    │                     │
  │────────────────►│ hold local transport stopped, status Connecting{D, s}        │
  │                 │ Listen{from:A,to:D,stream:s} ─►│ ─────►│ accept (policy, ≤ 8)  │
  │                 │                  │             │       │ start_stream_capture  │
  │                 │                  │             │       │ stream_open(A, s, ice)├─► offer
  │                 │                  │ ◄─ Signal{Offer} ◄──│ ◄── poll_stream ──────┤
  │ ◄ Event Signal{Offer}              │             │       │                       │
  │ setRemoteDescription, createAnswer │             │       │                       │
  │ SendSignal{Answer} ─►│ Signal{Answer} ─►│ ──────►│ stream_signal(A, s, Answer) ─►│
  │ ⇄ trickle ICE: SendSignal{Ice} / Event Signal{Ice} (both ways, same path)          │
  │ ═════════════════ ICE + DTLS: media flows P2P (or via TURN) ══════════════════════│
  │                 │ ◄─ StreamClock{rtp, position, ...} every 100 ms + on jumps ◄────│
  │ ◄ Event StreamClock; status Listening{D, s} (first clock); presence listening_to=D │
```

- **Refused** (hosting disallowed, no sender, `MAX_LISTENERS`, the host is itself
  listening): the host answers `Signal { Bye { reason } }`; the listener goes
  `Ended { host, reason }` and its local engine is back.
- **Stop**: `StopListening` → `Unlisten` to the host; the UI closes its peer connection; the
  host `stream_close`s (and `stop_stream_capture` after the last listener).
- **Failure**: the listener's UI sends `SendSignal { Bye }` when its peer connection fails
  (ICE `failed`, or no media for 10 s); the host's bridge reports
  `StreamOutput::State { Failed }` and the host sends `Bye`. Both end the stream; no
  automatic retry (the UI may offer one).
- **Host leaves / disconnects**: the relay broadcasts `Leave { host }`: the listener ends
  with "Diego left". A host's relay reconnect is a leave (streams do not survive it).
- **Listener leaves**: its `Leave` ends its stream on the host.
- **Stale signals**: every signal carries the listener-chosen `stream` id (random u32, new
  per `Listen`); signals for an unknown stream are dropped, so a late `Offer` from an old
  request never plays.
- **ICE servers**: after its sync (and every 6 h), the relay sends each site
  `IceServers { servers }` (§10); the controller emits `CollabEvent::IceServers` (settings
  override: `SetIceServers`), the UI endpoints use them for `RTCPeerConnection`, the native
  sender gets them in `stream_open`.

### 9.3 Host side rules

- Hosting policy (`SetHosting { allow, ui_sender, remote_transport }`): default allow + remote
  transport in a session. `can_host` (presence) = allowed ∧ a sender exists (native, or UI
  sender declared) ∧ not listening to someone else.
- A host that starts listening to someone else ends its own streams (`Bye` "host started
  listening elsewhere"): no chains.
- The stream carries whatever the host hears minus previews: when the host is stopped the
  listeners hear silence (and metronome only if it plays), exactly like the host.
- Clock anchors come from the tap headers (`StreamBlock { sample_time, position, playing,
  recording, bpm, latency, jump, gap }`), see §9.4. The web host builds them in the UI (§9.4).

### 9.4 Stream clock: what the listener's playhead shows is what it hears

The listener's playhead must show the audio it is hearing, which lags the host by network +
jitter buffer + output latency (typically 60-200 ms), varying over time. We map audio, not
wall clocks: every RTP packet carries an RTP timestamp in the **host's sample clock**
(48 kHz), and the browser tells the receiver which RTP timestamp it is playing.

Host → listener: `StreamClock { rtp, position, playing, recording, bpm, loop_enabled,
loop_region, metronome, discontinuity }` = "the sample with RTP timestamp `rtp` (in *this
listener's* RTP stream) is the timeline at `position`". Sent per listener (RTP timestamps
start at a random offset per stream) every `STREAM_CLOCK_INTERVAL_MS` (100 ms) and
immediately at every discontinuity (play, stop, locate, loop wrap, latency change, tap
gap), with `discontinuity: true` and the `rtp` at which the first post-jump sample is
**heard** (the jump sample plus the graph latency; below).

Host math (native). For each tapped block the sender knows (`StreamBlock`): its engine
sample time `T`, the timeline position `P` rendered at `T`, `playing`, `jump`, the graph
latency `L` (samples, PDC) and the engine rate `sr`. The engine renders the timeline at `T`
but, because of PDC, **that material leaves the outputs (and the tap) `L` samples later**:
the tap sample at `T + L` is the timeline at `P`. With the stream's 48 kHz sample counter
`n(t)` (after resampling), the stream's random RTP offset `o`, and `L48 = round(L * 48000 /
sr)`:

```
rtp(t)        = o + n(t)                  (mod 2^32; str0m stamps packets from the same counter)
anchor(block) = { rtp: rtp(T) + L48, position: P, playing, discontinuity: jump }
```

This one rule holds for every anchor, periodic or not. For a **jump** (`StreamBlock::jump`:
play from stopped, locate, loop wrap; set by the engine on the first sub-block after it),
the anchor lies `L48` in the **future** of the jump sample: for the first `L` samples after
a jump, the tap still carries the pre-jump timeline (the PDC delay lines are still
emptying), and listeners keep mapping them with the previous anchor, which is exactly
right. Worked example (48 kHz, `L` = 480 samples = 10 ms, 120 bpm = 2 beats/s, loop
0..8 beats, `o` chosen so that `rtp(T) = 1_000_000` at the wrap):

| engine block | tap audio at that time | anchor sent |
|---|---|---|
| `T - 4800`, `P` = 7.8 | timeline ≈ 7.78 | `{ rtp: 995_680, position: 7.8 }` |
| `T` (wrap: `P` = 0, `jump`) | timeline ≈ 7.98 (pre-jump, still in PDC) | `{ rtp: 1_000_480, position: 0.0, discontinuity }` |

The listener playing `r = 1_000_300` uses the first anchor: `7.8 + (1_000_300 - 995_680) /
48000 s × 2 = 7.9925` (just before the loop end, what it hears); from `r = 1_000_480` on it
uses the second: 0.0. Anchoring the wrap at `rtp(T)` instead would show 0.0 at 1_000_000
while the listener still hears 7.99, i.e. map the wrap to `loop_start - L`.
A change of `L` (a republished graph) is sent as a `discontinuity` anchor for the next
block. Stop is not a `jump` (the position just stops advancing): the anchor with `playing:
false` at `rtp(T) + L48` freezes the playhead when the last played sample is heard.

**Count-in** (recording with count-in bars): the host's transport really plays the pre-roll,
from `record start - count-in bars` up to the record start, so anchors carry `playing: true`,
`recording: true` and the pre-roll `position` (which may be negative), exactly like the
host's own playhead, plus `count_in_end: Some(record start)` while a count-in is armed
(`None` otherwise). A listener shows "count-in" while its mapped position is before
`count_in_end`, and hears the count-in clicks in the stream.

Encoder delay (native): Opus has a fixed algorithmic delay of 312 samples at 48 kHz
(6.5 ms). The native sender **pre-compensates RTP timestamps for it**: each Opus frame is
stamped `o + n - 312`, so that the *decoded* sample with RTP timestamp `o + m` is stream
sample `m`. The anchor formula above then holds for what the listener hears. A stop is sent
as an anchor with `discontinuity: true` (so a listener never extrapolates a playing anchor
past it), even though it is not a timeline jump. The native sender sets the encoder bitrate
directly (target 128 kbit/s, 48-192 kbit/s from the bandwidth estimate), since str0m cannot
put `maxaveragebitrate` in its SDP. Native host candidates are IPv4 only (default-route
interface and loopback).

Host math (web): the RTP timestamp is chosen by the browser. The UI sender observes it with a
read-only encoded transform on the sender (`RTCRtpScriptTransform`, or
`createEncodedStreams` on Chromium): each `RTCEncodedAudioFrame` has its RTP `timestamp`,
observed at `performance.now()`. The UI maps that instant to the engine's position through
its own playhead stream (`AudioContext.getOutputTimestamp()` relates context and
performance time) minus the constant encoder pipeline delay (one 20 ms frame + 10 ms).
Accuracy ±10 ms (under one display frame). The latency rule above applies unchanged: the
worklet's tap output carries the timeline `L` samples after it was rendered, so the UI
anchors a jump at the instant its first post-jump sample reaches the tap output (render
time + `L`), never at the render time. A browser without encoded transforms cannot host
(`can_host` false).

Listener math (UI, per animation frame):

```
ssrc      = receiver.getSynchronizationSources()[0]     // { rtpTimestamp, timestamp }
r_now     = ssrc.rtpTimestamp
          + round((performance.now() - ssrc.timestamp) / 1000 * 48000)   // extrapolate
          - outputLatencySamples            // AudioContext.outputLatency when known, else 0
A         = latest anchor with (r_now - A.rtp) as i32 >= 0      // wrapping u32 arithmetic
dt        = ((r_now - A.rtp) as i32) / 48000                    // seconds of audio since A
position  = A.playing ? beats_at(seconds_at(A.position) + dt) : A.position
            // with the replicated tempo map; A.bpm only as a fallback
if A.playing && A.loop_enabled && A.position < loop_end && position >= loop_end:
            position = loop_start + (position - loop_start) mod (loop_end - loop_start)
            // predicted wrap; the wrap anchor (discontinuity) makes it exact
```

- Anchors are kept ordered by `rtp` (≤ 32; older ones than the one in use are dropped). They
  usually arrive **before** their audio plays (the relay path is not slower than the jitter
  buffer); one arriving late applies from then on. A `discontinuity` anchor is never
  interpolated across: the position jumps exactly when its first sample plays.
- **Drift**: none in the mapping, since it only uses the host's sample clock (RTP) and the
  browser's playout report; the host/listener clock drift itself is absorbed by the WebRTC
  jitter buffer (time-stretching), not by us.
- **Wrap**: RTP timestamps wrap every 24.8 h at 48 kHz: always compare as `i32` differences.
- Transport display: while listening, `Event::Transport` reflects the latest applied anchor
  (playing, recording, loop, bpm, metronome of the host) and the UI playhead uses the mapped
  position, ignoring the local engine's `PlayheadUpdate`s.
- Fallback when `getSynchronizationSources()` has no `rtpTimestamp`: position = latest anchor
  extrapolated by local time minus the receiver's `jitterBufferDelay / jitterBufferEmittedCount`
  (from `getStats`) and half the RTT: degraded but usable.

### 9.5 Transport while listening

- **The listener's local engine transport is held stopped** from `Listen` until the stream
  ends. Its master is not muted: with the timeline stopped, only local previews (browser
  preview, drum pad audition, live MIDI on armed tracks, monitoring) sound, which is what we
  want. The controller never sends `Play` to the local engine while listening.
- Forwarded to the host as `TransportRequest` (reply `Unit` at once; the result shows when
  the host's next anchor arrives): `Play`, `Stop`, `TogglePlay`, `Locate`,
  `SetLoopEnabled`, `SetLoopRegion`.
- Not forwarded: `SetMetronome` (a local setting: the listener hears the host's click, its
  own setting applies after listening), `SetTempo`/`SetTimeSignature`/`TapTempo` (document
  edits: they replicate as usual). Recording while listening is refused (`InvalidState`,
  "stop listening to record").
- **Who wins**: the host applies requests exactly like its own transport commands, in
  arrival order. The relay is one ordered stream into the host, so requests from several
  listeners are totally ordered, and the host's own commands interleave by when the host
  handles them: **last command handled by the host wins**, no locks, no ownership (a shared
  transport, like a band). Forwarded loop changes are site-local settings on the host,
  applied outside its undo history (like any remote change).
- **The host throttles requests itself** (it cannot trust listeners to): per listener at
  most 20 requests/s; beyond that it keeps only the latest `Locate` and the latest loop
  change and drops the rest (the relay's 200/s bucket is only the outer bound).
- Ignored by the host: requests while it records or counts in (only the host stops its own
  recording), requests from sites that are not its current listeners (stream id mismatch),
  and all requests when `remote_transport` is off.
- When the stream ends (stop, host left, refusal, failure): the local transport stays
  **stopped**, located at the last heard position; it never auto-plays. The status goes
  `Off` (explicit stop) or `Ended { reason }`.

### 9.6 Plugins while listening (node `plugin-mirror`, later)

The host's engine processes the audio, so a listener hears the host's plugins even when it
does not have them. The listener's own engine keeps its instances (the timeline is stopped).
A listener that has the plugin installed may open its GUI as a **mirror**: a GUI-only
instance, never routed, never processing (`EngineBridge::create_plugin_mirror`,
`destroy_plugin_mirror`, `set_plugin_mirror_param`, defaulted `Unsupported`). Its GUI edits
come back through `poll_plugins` as `ParamEdited`: ordinary undoable, replicated `SetParam`
ops, which the host's live instance receives like any remote edit (so the listener hears the
change). Document param changes are pushed into the mirror. Opaque state (a preset loaded in
the mirror GUI) replicates through the save-time capture of §2.2, reading the mirror's state.
`OpenEditor` for a device with a mirror and no live instance opens the mirror. A listener may
also (setting) swap its live instances for mirrors while listening, to save CPU.

### 9.7 Security

- **Only session members signal each other**: site-to-site messages are accepted from
  synced, token-authenticated connections only, `from` must be the sender's own site, `to`
  must be a synced peer of the **same** session, and the message goes to `to` only (never
  broadcast, logged or cached). `IceServers` is relay → site only. SDPs ≤ 32 KiB, candidates
  and reasons ≤ 1 KiB, the token bucket of §8.5. (`relay/mod.rs`, tests in
  `relay/tests.rs`.)
- **No unsolicited audio**: a listener only accepts an `Offer` for the `stream` it asked for,
  from the host it asked; hosts only accept answers/ICE for streams they created.
- **Media is end-to-end encrypted** (DTLS-SRTP) even through TURN; the relay sees
  ciphertext. DTLS fingerprints travel through the relay, which is trusted like it already
  is for the document.
- **ICE candidates reveal IP addresses** to the session's members (browsers use mDNS for
  local addresses). A "relay only" setting (`iceTransportPolicy: "relay"`) hides them when
  TURN is available.
- TURN credentials and limits: §10. No unauthenticated media relay.
- Autoplay: the "Listen" click is the user gesture that starts playback.

## 10. NAT traversal: the relay is the STUN (and optionally TURN) server

No third party: nothing points at Google/Cloudflare STUN by default. The only ICE servers
are the relay's, or the ones set in settings (`SetIceServers`; e.g. a self-hosted coturn).

- **Port**: the relay binary also binds **UDP on the same port number** as its WebSocket
  (TCP) listener, so a relay stays one host:port (dev: `ETHER_DEV_PORT + 3`, both
  protocols). `--no-stun` disables it.
- **STUN** (always on): an RFC 8489 Binding responder (XOR-MAPPED-ADDRESS, FINGERPRINT) on a
  small thread, using the `stun` crate's message codec (webrtc-rs). Binding requests are
  unauthenticated by design, and a response is **bigger** than a minimal request: a
  20-byte attribute-less request (what browsers send to gather srflx candidates, so it
  cannot be dropped) gets 40 bytes (IPv4: header + XOR-MAPPED-ADDRESS + FINGERPRINT) or 52
  bytes (IPv6), an amplification of at most **2.6×** (no SOFTWARE or other optional
  attribute is ever added). Bounds: 50 responses/s per source IP, and a **global cap of
  1000 responses/s** for the whole responder (≤ 52 KB/s of reflected traffic); requests
  over either limit, malformed, or not Binding requests are dropped silently.
- **TURN** (optional, cargo feature `turn` of `ether-collab`, `--turn`): the webrtc-rs `turn`
  crate server (UDP allocations, RFC 8656) in its own tokio runtime thread; when on, it owns
  the UDP port and answers Binding requests itself. Relayed ports from a configurable range
  (`--turn-ports`), public address from `--public-ip`.
- **Credentials** (TURN REST API scheme, per site, time-limited), only for token-protected
  relays:
  - `secret` = 32 random bytes from the OS, generated when the relay starts, kept in memory
    only, **never sent to anyone and not derived from the relay token** (every site holds
    the relay-wide token, so a token-derived secret would let any member mint credentials
    for any site id and any expiry). A relay restart invalidates outstanding credentials;
    sites get fresh ones with the `IceServers` that follows their re-sync;
  - `username = "<expiry unix seconds>:<site id>"`, TTL 12 h, minted by the relay for the
    site of the connection it sends them to;
  - `credential = base64(HMAC-SHA1(secret, username))`;
  - the TURN auth handler recomputes it and rejects a username whose expiry is past, or
    **later than now + 12 h + 60 s** (clock skew), or that does not parse. A relay without a
    token (loopback dev) serves STUN only.
- **Advertisement**: after a site's sync, the relay sends `IceServers` built for that site
  (`Relay::set_ice_provider`), and again every `RelayConfig::ice_refresh_ms` (6 h) so
  credentials never expire mid-session. URLs use `--public-host`, else the host name the
  site used to reach the relay.
- **Limits**: ≤ 4 allocations per username, ≤ 64 per relay, 512 kbit/s per allocation,
  lifetime ≤ credential expiry, and denied peer addresses: loopback, link-local, private
  ranges and the relay's own addresses (no SSRF into the relay's network), unless
  `--turn-allow-private` (LAN tests).
- Future: TURN over TCP/TLS (`turns:`) for UDP-blocked networks.
- The native sender (str0m) gathers its own host candidates and a server-reflexive one
  (a Binding request to the advertised STUN URL on its socket); it does not need its own
  relay candidate, because the listener's TURN allocation is reachable from anywhere.

## 11. Library choices

| Need | Choice | Why (and rejected alternatives) |
|---|---|---|
| Browser send/receive | the browser's `RTCPeerConnection` | Jitter buffer, PLC, drift, congestion control, Opus: not reinvented. |
| Native sender | **`str0m`** (MIT/Apache) | Sans-IO and synchronous: fits the thread-based native host (no async runtime), one sender thread and one socket, deterministic tests with fake time; small dependency tree; RTP timestamps under our control (exact stream clock); has TWCC bandwidth estimation. **Rejected: `webrtc-rs`** (a Pion port: tokio everywhere, large tree, API churn, itself moving to a sans-IO rewrite). str0m does not gather srflx/relay candidates: one Binding request on the same socket covers srflx (§10). DTLS backend: prefer a feature that avoids a system OpenSSL (decided in `stream-host`). |
| Opus encode | **`opus`** crate (bindings to libopus via `audiopus_sys`, which builds the vendored libopus statically) | Reference codec, BSD-3 (GPL-compatible), no system library needed. |
| Tap → sender | `rtrb` | Already the engine's SPSC ring; RT-safe. |
| Resampling to 48 kHz | `rubato` (workspace) | Already used by `ether-media`. |
| STUN | `stun` crate (webrtc-rs) message codec | Sync encode/decode; a Binding responder is ~100 lines around it. |
| TURN | `turn` crate (webrtc-rs), feature-gated, tokio in its own thread | Maintained RFC 5766/8656 server with a TURN-REST-style auth hook, in-process (the relay stays one binary). Rejected: coturn (C, a second process to deploy), writing TURN ourselves (large security surface). |
| Topology | mesh host → listeners | ≤ 8 listeners; an SFU is future work. |

## 12. Social: chat, notes, peer playheads (base-62, node `collab-social`)

Four owner requests, one node. Chat and notes are **document** state (saved with the
project, replicated as ordinary ops); peer playheads are **presence** (ephemeral); the
"hide others" toggle is **UI state** (per user, never sent). Contract: `ether_model::social`
(entities, caps, `is_untracked`), `ether_protocol::social` (`ChatCommand`,
`PinnedNoteCommand`), `ether_protocol::collab` (`PresenceState::transport`, `PeerTransport`,
`CollabEvent::ChatReceived`). Until the node lands, `Chat::*` and `PinnedNote::*` reply
`Unsupported` (engine: `ether-controller/src/social/mod.rs`, pinned in
`tests/social_prewire.rs`; mock: `ui/src/transport/mock/roadmap/social.ts`).

### 12.1 Chat (a project journal)

- **Entity** `ChatMessage { id, seq, author, text, sent_at }` in `Project::chat`
  (`#[serde(default)]`: `.ether` v3 files without it load unchanged, no version bump).
  `id` is a client-chosen ULID (unique across sites). `author = Author { name, site,
  actor, color }` is a **snapshot** taken by the sender's controller (session name, site,
  its relay colour at the time), so the journal keeps showing who wrote what after the
  session. `text`: 1..=2000 chars (`CHAT_TEXT_MAX_CHARS`, counted in `char`s) and ≤ 4096
  UTF-8 bytes (`TEXT_MAX_BYTES`), not only whitespace. `sent_at`: unix ms on the sender's clock, display only.
- **Order = relay log order, not `sent_at`.** `seq` is assigned by `Project::apply`: an
  `Insert` with `seq: 0` gets `max(seq) + 1` (saturating: a forged `u64::MAX` neither
  panics nor wraps; ties sort by id). The confirmed document is the resolve-fold of
  the log (§2), so every replica numbers messages in log order; a pending own message gets
  a provisional `seq` that is recomputed on every rebase (it re-applies the op as sent, with
  `seq: 0`), and its echo lands it where the relay put it. A non-zero `seq` is kept (inverse
  ops, snapshots). `Project::chat_ordered()` sorts by `(seq, id)`.
- **Not undoable.** `History::commit` applies chat ops (`Insert`/`Remove` of a
  `ChatMessage`; `social::is_untracked`) but never records them: a chat-only transaction
  pushes no step and keeps the redo stack and the open gesture; in a mixed transaction only
  the other ops form the step. The controller still stamps and sends the transaction (the
  `edit_with` path: `collab_local_commit` gets the full applied ops and inverse, so
  rebasing works as for any pending transaction). `Chat` is not a document command: it is
  refused inside `Edit::Batch`. Test: `ether-model/tests/social.rs`.
- **Cap.** `CHAT_MAX_MESSAGES` = 2000. `Send` computes `Project::chat_overflow(cap)` on the
  live document and removes those oldest messages **in the same transaction** as the
  insert: removes are ordinary ops, so every replica removes the same ones. Two sites
  sending concurrently at the cap may both remove the same oldest message (the second
  `Remove` is skipped by resolve), leaving `cap + 1` until the next send prunes two: bounded
  and convergent. A peer that never prunes is stopped at apply time: `Project::apply`
  refuses an insert past `CHAT_HARD_MAX_MESSAGES` (2 × the cap). The confirmed state is the
  same on every replica, so every replica refuses the same insert (resolve skips it). Worst
  case stored: 4000 × 4 KiB, far below a snapshot's 16 MiB with the rest of the project.
- **Command** `Chat::Send { id, text }`: validate, fill `author` and `sent_at`, then one
  transaction `Insert ChatMessage { seq: 0 } + Remove overflow` (an existing `id` is a
  no-op). Outside a session: `InvalidState` (chat is hidden there). Replies `Unit`; the
  message reaches the UI as a patch, like any edit.
- **Incoming** messages arrive as peers' patches (`Patch::origin` set). The controller also
  emits `CollabEvent::ChatReceived { ids }` for peers' messages **sequenced after this
  site's join catch-up** (never for the join snapshot, the catch-up log, a project load, or
  own messages), so the UI toasts only live messages.
- **Protocol version 2.** New entity variants travel in transactions, which an older
  build cannot decode (it would drop them silently and diverge), so base-62 bumps
  `COLLAB_PROTOCOL_VERSION` to 2: the relay refuses mixed-version sites at the hello (§4).
- **Authorship is verified on receipt** (implemented: `social::is_forged_chat`, applied in
  `collab_apply_remote` and in the debug echo check): a peer's `Insert ChatMessage` is
  dropped unless `author.site == Some(origin.site)` (the relay-verified sender, §7) and
  `seq == 0`. Deterministic (a function of the op and its stamped origin), so every
  replica drops the same ops, and a dropped message never produces `ChatReceived`. No
  legitimate path sends anything else, because chat is never undone or redone. Tested in
  `ether-controller/tests/social_sanitize.rs`. **Note authorship is best-effort**: undoing a
  discard legitimately re-inserts another user's note, so notes cannot use this rule; their
  `author` is what the inserting op says.
- **Own colour.** Today a site never learns its relay colour (the relay drops a site's own
  presence). The node adds it: the relay sends each site its own stamped default presence
  once synced (`relay/mod.rs`, next to the peers' presence it already sends a joiner), and
  the controller records its colour from a `Presence` with its own site instead of
  dropping it (`collab/mod.rs`, one line). `Author::color` is `None` until known.
- **Older readers.** A build without these tables (`.ether` v3 readers) ignores the unknown
  `chat` and `pinned_notes` keys and loses them on its next save. Acceptable because the v4
  bump (contracts-3, PR #106) ships in the same release: a v3 reader then refuses the file
  outright instead of silently dropping the journal. In a session, older builds are
  refused by the collab protocol version (above).
- **Solo** (no session): the chat UI is hidden; messages still load with the project, stay
  in the file, and show again in the next session.

UI (`ui/src/features/collab/social/**`):
- A **Chat** section in the left sidebar: a new rail tab (`LeftTab` `"chat"`, `LEFT_TABS`
  entry, `LeftPanel` case), shown **only in a session**. Message list in `chat_ordered`
  order (author name in the author's snapshot colour, relative time from `sent_at`), input
  at the bottom (Enter sends, Shift+Enter new line, a live counter near the 2000 cap).
- **Toasts** while the chat section is closed: each `ChatReceived` id pops a toast at the
  top right (author, first line, click opens the chat), a few at most on screen, auto
  dismissed. The kit has no toast: the node adds one as its own kit component
  (`ui/src/kit/Toast.tsx` + one export line in `kit/index.ts`), tokens only.
- **Shortcut**: `Mod+Shift+M` opens the chat section if needed and focuses its input (listed
  in the command palette as "Chat: Focus input"; Escape returns focus to where it was).
  Only in a session. The node checks the existing keymap for conflicts.

### 12.2 Notes pinned wherever cursors are tracked (arranger and piano roll)

- **Entity** `PinnedNote { id, position, text, author, created_at, resolved }` in
  `Project::pinned_notes` (`#[serde(default)]`; named `PinnedNote` because `Note` is the MIDI
  note). `text`: 1..=2000 chars and ≤ 4096 UTF-8 bytes (`NOTE_TEXT_MAX_CHARS`,
  `TEXT_MAX_BYTES`). `resolved` defaults to `false`. At most `MAX_PINNED_NOTES` (500) per
  project: an insert past it is refused by `Project::apply` ("a project holds at most 500
  notes"; the UI shows it). Validated by `Project::apply` (text, author, position ranges,
  count).
- `position = NotePosition { beats, track, y, editor? }`, the same coordinates as a
  presence pointer (§8.3), so a note can be left **wherever cursors are tracked**:
  - **Arranger** (`editor: None`): `beats` ≥ 0; `track` = the row, `y` in 0..=1 inside it
    (including its expanded lanes); `track: None` = off-track, with `ArrangerPointer::y`'s
    meaning: `y` = 0 over the ruler/header area, `y` > 0 below the last track as the
    fraction of the free space there (each user maps it onto their own).
  - **Piano roll** (`editor: Some(EditorNotePosition { clip, beats, pitch })`, mirroring
    `EditorPointer`): content beats of that clip (≥ 0) and `pitch` in 0..=128 (60.5 = the
    middle of C3's row). The arranger fields are then `beats: 0, track: None, y: 0`
    (validated) and ignored.
- `track` and `clip` are **weak references**: never validated, never cascaded, never block
  a delete. A note whose track is gone shows in the ruler row at its `beats`; one whose
  clip is gone is not shown. Undoing the delete puts it back in place.
- **Commands** `PinnedNote::{Add { id, position, text, author_name }, Edit { id, text?,
  position?, resolved? }, Delete { ids }}`: document commands (undoable, replicated, allowed
  in a `Batch`, work outside a session). The controller fills `author` (in a session: the
  session identity; outside: `author_name` or "", no site/colour) and `created_at`. **Any
  user can edit, resolve, move or discard any note**, for everyone; discarding is undoable
  (by whoever discarded it, per-site undo §3). Authorship is best-effort (§12.1): an undo
  legitimately re-inserts another user's note, so receivers cannot verify it like chat.
- UI (`ui/src/features/collab/social/notes/**`), the owner's context-menu pattern in both
  places:
  - **Arranger**: **"Leave a note"** in the lane/ruler menus of `ArrangementView.tsx` and
    `TrackRow.tsx`, at the right-click position mapped to song coordinates with
    presence-v2's `coords.ts`; dots drawn in an overlay layer over the arrangement (the
    PresenceLayer pattern: one layer, pointer-events only on the dots, each user's own
    layout maps song → screen).
  - **Piano roll**: **"Leave a note"** in the note-grid context menu (`NoteGrid.tsx`), at
    the right-click position in content coordinates (the `EditorPresence` mapping); dots
    in an overlay over the grid (mounted in `PianoRoll.tsx`), shown only for the open clip.
  - Both: a small dot in the author's colour; hover/click shows the text, collapsed to a
    few lines with "more" when long; edit, resolve and "Discard note" from the dot's menu;
    drag the dot to move it (one gesture). Resolved notes are dimmed.

### 12.3 Peers' playheads

- Transports are per site (§1), so each peer has its own playhead. Our presence carries it:
  `PresenceState::transport: Option<PeerTransport { position, playing, sent_at_ms,
  loop_region }>` (additive, `#[serde(default)]`, omitted when `None`). It is
  **controller-owned** (`social_presence`, next to `listening_to`/`can_host`): whatever the UI
  sends is overwritten. `loop_region` is `Some` while loop playback is on.
- Published within the presence throttle (≤ 10 Hz): at once on play, stop, locate, loop
  and tempo-map changes, and every `PEER_TRANSPORT_REFRESH_MS` (1 s) while playing (receivers
  extrapolate in between). `None` while this site listens to a host (§9: the host's
  playhead is the one heard, shown by the listening badge).
- Receivers (UI, per animation frame): a new `sent_at_ms` marks a new sample; record its
  **local arrival time** (clocks are not synchronized, so `sent_at_ms` is never compared with
  the local clock). While `playing`: `position` + the time elapsed since arrival, converted
  with the **replicated tempo map** (`seconds_at`/`beats_at` from `position`), then, if
  `loop_region` is set and `position` was inside it, wrapped into it; if a refresh is late
  by more than 2 × the refresh interval, hold the last extrapolated position (do not run
  away). Stopped: `position` as is.
- UI: one line per peer over the arrangement (and a small cap on the ruler) in the peer's
  colour, visibly distinct from our own playhead (thinner, dashed or with the peer's
  initials on the ruler cap), in the presence overlay layer (the PresenceLayer pattern,
  pointer-events: none), hidden when off screen.

### 12.4 "Hide others" (local preference)

- A toggle in the collab ("jam") dialog, **"Hide others"**: while on, this user sees **no
  peers' pointers, playheads, selection outlines, presence chips on the timeline and no
  pinned notes**. Nothing else changes: peers' edits still apply, the chat and its toasts
  still work, the participant list in the dialog and the top-bar chips stay (they are how to
  turn it back off), and our own presence is still published.
- UI state only: stored in local settings (`localStorage`, next to the dialog's remembered
  join fields), never replicated, never a command. One selector (`useHideOthers()` in the
  collab store) that **every** presence and notes renderer honours: `PresenceLayer`,
  `EditorPresence`, `PeerHighlights` (selection outlines), the playhead overlay, the notes
  overlay and the "Leave a note" menu entry (hidden while hiding notes).

## 13. Code layout

- `ether-collab` (native + wasm): wire helpers (snapshot/version encoding, media chunking,
  binary frames), `CollabTransport` implementations: native WebSocket client thread
  (tungstenite), wasm `web_sys::WebSocket` (inside the controller Worker), in-memory loopback
  for tests; the relay (`relay` module, native only) and the `ether-collab-relay` binary.
- `ether-controller/src/collab/`: session state (site, seq, pending, presence), `resolve`
  (resolve, cascades, value-guarded undo, local-only filter), rebase, stamping hook in
  `edit_with`/undo/redo (`handlers.rs`), media push/receive, tick polling. The controller opens the connection through
  `ether_collab::connect(url)` by default; tests inject a connector
  (`EtherController::set_collab_connector`). No host crate changes.
- `ether-model/src/history.rs`: `commit_with_inverse` (the transaction's own inverse, even
  when merged into a gesture step); `undo_with`/`redo_with`.
- Running a relay: `cargo run -p ether-collab --bin ether-collab-relay` (listens on
  `$ETHER_COLLAB_PORT`, else the dev instance's `$ETHER_DEV_PORT + 3`, else an OS-chosen
  port; prints its URL and a generated token; `--token`, `--listen`, `--no-token` for
  loopback-only use).
- Tests: `crates/ether-collab` (relay state machine, wire), `crates/ether-controller/tests/
  collab.rs` (convergence property test over 3 sites with random concurrent edits, undo,
  cascades, delivery orders, dropped links (reconnect with pending ops), leave + re-join
  on the same controller and relay compaction, plus a fresh replay site; per-site undo;
  late join; media; reconnect with pending ops; re-join on the same controller; rejoin
  backup; rebase keeping derived data; plugin state; local-only state), `collab_relay.rs` (two controllers through the real
  relay over WebSockets, bad token), `apps/web/e2e/collab.spec.ts` (two browser contexts).
- UI: `ui/src/features/collab/**` (`PresenceBar`, store, peer selection outlines injected as
  a `<style>` keyed on `[data-track]`/`[data-clip-id]`), mock simulation `MockCollab` in
  `ui/src/transport/mock/roadmap/collab.ts` (session status, simulated peers).
- base-53 (§8-§10): `ether-protocol/src/collab.rs` (types), `ether-collab/src/relay/`
  (`mod.rs` routing + ICE advertisement, `limits.rs` throttles), `ether-core/src/
  stream_tap.rs` (+ the tap call in `engine.rs`), `ether-controller/src/streaming.rs`
  (bridge types) and the defaulted `EngineBridge` stream/mirror hooks (`lib.rs`), the
  per-node controller modules `collab/{presence, listen, stream_host}.rs` (dispatched from
  `collab/mod.rs`; the transport intercept in `handlers.rs::transport_command`), tests
  `ether-protocol/tests/collab_v2_shapes.rs`, `ether-core/tests/stream_tap.rs`,
  `ether-controller/tests/collab_prewire.rs`. Node boundaries: docs/ROADMAP.md.
- base-62 (§12): `ether-model/src/social.rs` (entities, caps, `is_untracked`, used by
  `history.rs`), `ether-protocol/src/social.rs` (commands), `ether-controller/src/social/`
  (stubs; `social_presence` called from `collab_flush_presence`), tests
  `ether-model/tests/social.rs`, `ether-protocol/tests/social_shapes.rs`,
  `ether-controller/tests/social_prewire.rs`; mock `ui/src/transport/mock/roadmap/social.ts`.
- Limitations: `wss://` works from the browser; the native client speaks `ws://` only (put a
  TLS proxy in front of a public relay). The relay keeps sessions in memory (a relay restart
  makes the first site to reconnect re-create the session from its replica).

## 14. Base changes

Landed in base-36: `CollabMessage::Media`, `CollabCommand::Get` (re-emits
`CollabEvent::Session` + `CollabEvent::Presence`, replies `Unit`), ownership of this file.
base-39: `MockTransport.ts` wiring of `MockCollab`. base-44: `engine.rs`
(`EngineState::request_reload_from_doc`).
base-53: presence v2 and listen-on-peer contract (§8-§11): protocol types, relay routing,
limits and ICE advertisement, the engine stream tap, the `EngineBridge` stream and
plugin-mirror hooks, controller dispatch into per-node stubs.
base-62: social contract (§12): chat and pinned-note entities, their commands, the chat
History exemption, `PresenceState::transport`, `CollabEvent::ChatReceived`, stubs.
