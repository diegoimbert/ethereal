# Real-time collaboration (`collab` node)

Status: design, proposed to the manager before implementation. Contracts it relies on:
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
- **Transport (play/stop/locate/record/arm) is per-site** and never shared. Everything in the
  document (including settings such as loop region, metronome, swing) is shared.

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
  Without pending transactions (the common case) this is just step 2. The echo of an own
  transaction only pops it from `pending` (the live doc already contains it in the
  right place).
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
- Everything else that is not an op stays site-local and derived: media `frames` fix-ups,
  plugin param mirroring after load, peaks, caches.
- ID minting (`copy_rack_pads`, duplicates, `SliceCommand::Auto`) is not a hazard: we sync
  **ops**, which carry the ids the author minted; commands are never replayed on another
  site. Resolve never mints ids (cascades only remove/detach).

## 3. Per-site undo

Undo only ever reverts **own** transactions: remote transactions never enter the local
`History`. With peers editing, a naive inverse would clobber their work, so undo/redo go
through `History::undo_with/redo_with` (new, `history.rs` is granted) with a collab
resolver:
- The collab module counts applied remote ops and remembers, per field
  (`(EntityKey, field)`; `Param` includes the param id; `Settings` fields too), the counter
  of the last remote op that touched it. Each undo step carries the counter at its (last)
  commit.
- Undoing a step: an inverse `Update` whose field was touched by a remote op after the step
  is **skipped** (the peer's later value wins). Inverse `Insert`/`Remove` go through
  resolve (re-inserting into a parent deleted by a peer is skipped; removing an entity a
  peer added children to cascades). The ops actually applied become a new stamped
  transaction; redo is the inverse of what undo applied.
- Outside a session `History::undo/redo` behave exactly as today.

## 4. Join, late join, reconnect, leave

- `CollabCommand::Join { server, session, token, name }`: the site generates a `SiteId` from
  host entropy (kept for the controller's lifetime, so a reconnect is the same site) and
  connects to `server` (`ws://host:port/`; the session name is the URL path
  `/<session>` percent-encoded). Handshake = remote-engine's `ClientHello`/`ServerHello`
  (token, protocol version), then `CollabMessage::Hello { site, actor, name, ... }`.
- Relay → joiner:
  - empty session: `SyncRequest { site: joiner, version: [] }` = "you create it": the site
    uploads its media (`Media` chunks) and a `Snapshot` of its open project. Other joiners
    wait until the snapshot arrives.
  - existing session: the media blobs, the latest `Snapshot`, the log after it, the other
    peers' `Hello` and `Presence`. The joiner saves its current project if dirty, writes the
    session project into its own store under the **same `ProjectId`**, opens it (media is in
    its `media/` already), and applies the log. Its own undo history starts empty.
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
- `ether-controller/src/collab/`: session state (site, seq, pending, touched fields,
  presence), resolve, rebase, stamping hook in `edit_with`/undo/redo (`handlers.rs`), media
  push/receive, tick polling. The controller opens the connection through
  `ether_collab::connect(url)` by default; tests inject a connector
  (`EtherController::set_collab_connector`). No host crate changes.
- `ether-model/src/history.rs`: `commit` also returns the transaction's own inverse;
  `undo_with`/`redo_with` + a per-step mark.
- UI: `ui/src/features/collab/**`, mock simulation in `ui/src/transport/mock/roadmap/collab.ts`
  (fake peers with presence, remote edits as patches with `origin`).

## 9. Needs from the base (BCR)

1. `CollabMessage::Media { file: String, hash: String, offset: u64, total: u64, data: Base64Bytes }`.
2. `CollabCommand::Get`: re-emit `CollabEvent::Session` + `CollabEvent::Presence` (UI mount /
   reload while in a session), reply `Unit`.
3. Ownership: `docs/COLLAB.md`.
