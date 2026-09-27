/**
 * Mock of `Collab::*` (docs/COLLAB.md). Owned by `collab`.
 *
 * `MockCollab` simulates a session: `Join` goes online at once with one simulated peer
 * ("Mock peer"), `SetPresence` is stored, `Get` re-emits the state, `Leave` goes offline.
 * `simulatePeer` lets tests and demos move a peer's presence (join/update/leave) and
 * `simulatePeerEdit` applies document commands as if a peer made them (the patch is a normal
 * mock patch; the mock has a single history, so it is only an approximation of per-site
 * undo).
 */

import type { CollabCommand, CollabStatus, Command, Presence, PresenceState, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";
import type { MockHost } from "./host";

const UNIT: ReplyValue = { type: "Unit" };

/** The site id of the mock engine itself. */
export const MOCK_SITE = "1";

/** Same rule as the engine: 1-64 of `[A-Za-z0-9._-]`. */
export function validSessionName(s: string): boolean {
  return /^[A-Za-z0-9._-]{1,64}$/.test(s);
}

function checkJoin(c: Extract<CollabCommand, { type: "Join" }>) {
  if (!validSessionName(c.session)) fail("InvalidArgument", "session names are 1-64 letters, digits, '.', '_' or '-'");
  if (!/^wss?:\/\//.test(c.server)) fail("InvalidArgument", "the relay URL must start with ws:// or wss://");
}

const emptyPresence = (): PresenceState => ({
  cursor: null,
  selected_tracks: [],
  selected_clips: [],
  selected_notes: [],
  selected_devices: [],
  view: null,
});

export class MockCollab {
  private status: CollabStatus = { type: "Offline" };
  private peers = new Map<string, Presence>();
  /** This site's last published presence. */
  presence: PresenceState = emptyPresence();

  constructor(private readonly host: MockHost) {}

  command(c: CollabCommand): ReplyValue {
    switch (c.type) {
      case "Join":
        checkJoin(c);
        this.status = { type: "Online", session: c.session, site: MOCK_SITE };
        this.peers = new Map([
          ["2", { site: "2", actor: null, name: "Mock peer", color: 0x5cffe8, state: emptyPresence() }],
        ]);
        this.emitAll();
        return UNIT;
      case "Leave":
        this.status = { type: "Offline" };
        this.peers.clear();
        this.emitAll();
        return UNIT;
      case "SetPresence":
        this.presence = c.presence;
        return UNIT;
      case "Get":
        this.emitAll();
        return UNIT;
    }
  }

  /** A peer joins or updates its presence (`state: null` = it leaves). */
  simulatePeer(site: string, name: string, color: number, state: PresenceState | null): void {
    if (this.status.type !== "Online") return;
    if (state) this.peers.set(site, { site, actor: null, name, color, state });
    else this.peers.delete(site);
    this.emitPeers();
  }

  /** Document commands as if a peer made them. */
  simulatePeerEdit(commands: Command[], label = "Peer edit"): void {
    this.host.applyDocument(commands, label);
  }

  private emitAll() {
    this.host.emit({ type: "Collab", event: { type: "Session", status: this.status } });
    this.emitPeers();
  }

  private emitPeers() {
    this.host.emit({ type: "Collab", event: { type: "Presence", peers: [...this.peers.values()] } });
  }
}
