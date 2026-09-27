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
- **Transport (play/stop/locate/record/arm) is per-site** and never shared. What else is
  local is listed in §2.1.

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
  pops it (and keeps its current inverse); a debug assertion and the tests check the
  equivalence.
- **Convergence invariant** (checked by the property test): on every site, after the log is
  fully delivered, `confirmed` = `fold(resolve, join snapshot, relay log)` and `pending` is
  empty, so `live` is identical everywhere (up to the site-local fields of §2.1). Every
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
| every entity table (tracks, clips, notes, devices + params, sends, lanes/points, tempo, signatures, warp markers, media, markers, MIDI mappings, drum pads) | transport: play/stop/locate/record, playhead, count-in |
| track mute, volume, pan, routing, names, colors, order | track **solo** (`TrackChange::Solo`), drum pad solo (`SetPadSolo`, runtime already), record-arm (runtime already) |
| settings: project name, swing, swing grid | settings: loop enabled + loop region, metronome on/off, volume, accent, sound, count-in bars |
| | MIDI learn mode/gestures, selection (shared only as presence), undo history |

Local-only ops are applied and undone locally as usual but filtered out of the stamped
transaction (a transaction with only local ops is not sent). Remote `Update`s of local
fields cannot arrive (senders filter them); a remote `Insert` carries the author's value,
which the receiver keeps (e.g. a new track is not soloed). A joiner keeps its own local
settings over the snapshot's and clears its track solos.

### 2.2 Writes that are not user edits

- Plugin GUI param edits (`ParamEdited`) already become undoable `Set Param` edits: they
  replicate like any edit.
- Save-time plugin state capture works on a copy of the document (`serialize`), so it never
  changes the document and never replicates. The session snapshot (creation and compaction)
  is built the same way, with live plugin states, so a joiner gets each plugin's current
  state once. Later opaque (non-parameter) plugin state changes don't replicate live; the
  next compaction snapshot carries them to later joiners.
- Missing plugins on a peer: the existing handling (the engine bypasses a device it cannot
  instantiate) applies. It is runtime state, never an op, so it never replicates.
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
  (token, protocol version), then `CollabMessage::Hello { site, actor, name, ... }`.
- Relay → joiner (after `Hello` + `SyncRequest`):
  - empty session: `SyncRequest { site: joiner, version: [] }` = "you create it": the site
    uploads its media (`Media` chunks) and a `Snapshot` of its open project. Other joiners
    wait until the snapshot arrives.
  - existing session: the media blobs, the latest `Snapshot`, the log after it, the other
    peers' `Hello` and `Presence`. The joiner saves its current project if dirty, writes the
    session project into its own store under the **same `ProjectId`**, opens it (media is in
    its `media/` already), and applies the log. Its own undo history starts empty.
  - **Existing local copy**: if the store already has that `ProjectId` and its saved content
    differs from the snapshot (unsynced local work, e.g. edits made offline after a
    previous session), the local copy is first duplicated as a new project named
    "<name> (local copy)" and a notification says so. Nothing is overwritten silently.
- `Snapshot.data` (opaque on the wire, defined by `ether-collab`): JSON
  `{ index, sites: { site: last_seq }, ether: "<.ether file JSON>" }`, base64.
- Reconnect (socket dropped, relay restart is out of scope): status `Connecting`, exponential
  backoff; on reconnect `Hello` + `SyncRequest { version = confirmed index (u64 LE) }`; the
  relay replays the log after that index (or snapshot + log if it was compacted), then the
  site re-sends its pending transactions with `seq` above the last own `seq` seen in the log.
  The relay drops a transaction whose `seq` is not above the last sequenced one of that site
  (duplicate after a lost ack).
- Log compaction: past `N` entries the relay sends `SyncRequest { site, version: [] }` to one
  site with no pending ops, which answers with a `Snapshot` of its confirmed state; the relay
  truncates the log before it.
- `Leave` (or disconnect): the relay broadcasts `Leave { site }`. The document stays open as a
  normal local project (dirty; saving it is the user's choice). Opening/creating/closing
  another project while in a session leaves the session.

## 5. Media

A site that imports audio (or records a take) **pushes** the file before the transaction
that references it: `Media { file, hash, offset, total, data }` chunks of 1 MiB, sent as
remote-engine binary frames (`BinaryKind::Bytes`, the JSON header carries `data: ""`). The
relay caches blobs by hash per session (bounded: `max_media_bytes`, default 2 GiB) and sends
them to late joiners before the snapshot. Because the relay keeps FIFO order, every site has
the file before the `Insert Media` arrives, so media never shows as missing. Receivers
stage chunks with the `ProjectStore` upload staging methods (in memory where the store has
none, e.g. OPFS today), verify `content_hash`, check `file` is a relative path under
`media/`, then `ProjectStore::write` it. Missing files at snapshot time are simply not sent
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
tokens; peer colors are data, like track colors).

## 7. Security

- Relay limits (all in `RelayConfig`): sessions per relay, sites per session, media cache per
  session and in total across sessions (chunks beyond it are still forwarded live but not
  cached for late joiners), log length before compaction and a hard log cap (past it the
  relay asks another peer for a snapshot and refuses new transactions from a site until
  the log shrinks: that site's link is closed and it resyncs on reconnect), and a per-site
  message rate limit (a site sending more than the cap per second is disconnected, like a
  slow reader). Presence is rate limited to 20 Hz per site (extra updates dropped).
- Relay handshake and limits copied from `ether-server`: shared token in `ClientHello`
  compared in constant time, token required unless bound to loopback, `Host`/`Origin`
  loopback check when there is no token, handshake deadline + 64 KiB pre-auth limit +
  bounded pending handshakes, idle ping/timeout, write timeout, max sites per session and
  max sessions. Post-auth message limit 16 MiB (snapshots).
- The relay rewrites/validates identity: `Transaction.origin.site`, `Presence.site` and
  `Leave.site` must be the sender's `Hello` site (else dropped).
- Receivers treat every remote op as untrusted input: it only enters the document through
  `Project::apply` (full validation); media paths are validated and hashes verified.
- No new unauthenticated surface: the UI↔engine protocol only gains the reserved
  `CollabCommand`s; the relay is a separate, token-protected listener.

## 8. Code layout

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
  cascades and delivery orders; per-site undo; late join; media; reconnect with pending
  ops; rejoin backup; local-only state), `collab_relay.rs` (two controllers through the real
  relay over WebSockets, bad token), `apps/web/e2e/collab.spec.ts` (two browser contexts).
- UI: `ui/src/features/collab/**` (`PresenceBar`, store, peer selection outlines injected as
  a `<style>` keyed on `[data-track]`/`[data-clip-id]`), mock simulation `MockCollab` in
  `ui/src/transport/mock/roadmap/collab.ts` (session status, simulated peers).
- Limitations: `wss://` works from the browser; the native client speaks `ws://` only (put a
  TLS proxy in front of a public relay). The relay keeps sessions in memory (a relay restart
  makes the first site to reconnect re-create the session from its replica).

## 9. Base changes

Landed in base-36: `CollabMessage::Media`, `CollabCommand::Get` (re-emits
`CollabEvent::Session` + `CollabEvent::Presence`, replies `Unit`), ownership of this file.
