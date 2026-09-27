/**
 * Mock of `Collab::*` (docs/COLLAB.md). Owned by `collab`.
 *
 * `MockCollab` simulates a session: `Join` goes online at once with one simulated peer
 * ("Mock peer"), `SetPresence` is stored, `Get` re-emits the state, `Leave` goes offline.
 * `simulatePeer` lets tests and demos move a peer's presence (join/update/leave) and
 * `simulatePeerEdit` applies document commands as if a peer made them (the patch is a normal
 * mock patch; the mock has a single history, so it is only an approximation of per-site
 * undo).
 *
 * Hosting (`stream-host`, docs/COLLAB.md §9.3): `SetHosting { allow: true }` in a session makes
 * the mock peer listen to this site (`ListenStatus.listeners`, endpoint `Ui` when the UI
 * declared a sender, else `Engine` as if the mock had a native one); `allow: false` or
 * `Leave` ends it. `SendStreamClock` / `SendSignal` are accepted (and recorded) in a session.
 * `simulateListener` adds or removes listeners.
 */

import type {
  CollabCommand,
  CollabStatus,
  Command,
  ListenerLink,
  Presence,
  PresenceState,
  ReplyValue,
  StreamClock,
} from "@/generated";
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
  /** Hosting policy (`SetHosting`; defaults of a session). */
  hosting = { allow: true, ui_sender: false, remote_transport: true };
  /** Who listens to this site. */
  listeners: ListenerLink[] = [];
  /** Anchors sent with `SendStreamClock` (latest last). */
  clocks: { to: string; stream: number; clock: StreamClock }[] = [];

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
        if (this.listeners.length) this.setListeners([]);
        return UNIT;
      case "SetPresence":
        this.presence = c.presence;
        return UNIT;
      case "Get":
        this.emitAll();
        if (this.listeners.length) this.emitListenStatus();
        return UNIT;
      // base-53 (docs/COLLAB.md §8-§10): like the engine until presence-v2, stream-host and
      // stream-listen land (each node extends its cases).
      case "SetHosting":
        this.hosting = { allow: c.allow, ui_sender: c.ui_sender, remote_transport: c.remote_transport };
        if (this.status.type !== "Online") return UNIT;
        if (!c.allow) this.setListeners([]);
        else if (this.peers.has("2")) {
          // The mock peer listens; its endpoint follows the sender this UI declared.
          const endpoint = c.ui_sender ? "Ui" : "Engine";
          const current = this.listeners.find((l) => l.site === "2");
          if (current?.endpoint !== endpoint) {
            this.setListeners([...this.listeners.filter((l) => l.site !== "2"), { site: "2", stream: 1, endpoint }]);
          }
        }
        return UNIT;
      case "SendStreamClock":
        if (this.status.type !== "Online") fail("InvalidState", "not in a collaboration session");
        if (!Number.isFinite(c.clock.position) || !Number.isFinite(c.clock.bpm)) fail("InvalidArgument", "stream clock values must be finite");
        this.clocks.push({ to: c.to, stream: c.stream, clock: c.clock });
        if (this.clocks.length > 64) this.clocks.shift();
        return UNIT;
      case "SetPointer":
      case "Listen":
      case "StopListening":
        return fail("Unsupported", `${c.type} is not implemented yet`);
      case "SendSignal":
        if (this.status.type !== "Online") fail("InvalidState", "not in a collaboration session");
        return UNIT;
      case "SetIceServers":
        this.host.emit({
          type: "Collab",
          event: {
            type: "IceServers",
            servers: c.servers ?? [],
            source: c.servers ? "Settings" : "Relay",
          },
        });
        return UNIT;
    }
  }

  /** A peer joins or updates its presence (`state: null` = it leaves). */
  simulatePeer(site: string, name: string, color: number, state: PresenceState | null): void {
    if (this.status.type !== "Online") return;
    if (state) this.peers.set(site, { site, actor: null, name, color, state });
    else this.peers.delete(site);
    this.emitPeers();
    if (!state && this.listeners.some((l) => l.site === site)) this.setListeners(this.listeners.filter((l) => l.site !== site));
  }

  /** A peer starts (`stream` set) or stops (`null`) listening to this site. */
  simulateListener(site: string, stream: number | null): void {
    const rest = this.listeners.filter((l) => l.site !== site);
    if (stream === null) this.setListeners(rest);
    else if (this.status.type === "Online" && this.hosting.allow) {
      this.setListeners([...rest, { site, stream, endpoint: this.hosting.ui_sender ? "Ui" : "Engine" }]);
    }
  }

  /** Document commands as if a peer made them. */
  simulatePeerEdit(commands: Command[], label = "Peer edit"): void {
    this.host.applyDocument(commands, label);
  }

  private emitAll() {
    this.host.emit({ type: "Collab", event: { type: "Session", status: this.status } });
    this.emitPeers();
  }

  private setListeners(listeners: ListenerLink[]) {
    this.listeners = listeners;
    this.emitListenStatus();
  }

  private emitListenStatus() {
    this.host.emit({
      type: "Collab",
      event: { type: "ListenStatus", status: { listening: { type: "Off" }, listeners: [...this.listeners] } },
    });
  }

  private emitPeers() {
    this.host.emit({ type: "Collab", event: { type: "Presence", peers: [...this.peers.values()] } });
  }
}
