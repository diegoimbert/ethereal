/**
 * Mock of `Share::*` (docs/SHARING.md), so the share-ui, join-flow and recents-shared nodes
 * can build their UI before the engine side lands. Owned by `share-engine` (base-115 stub).
 *
 * - `Start` → `Hosting` with an edit and a listen link (random room/keys on
 *   `invite_origin`), the user as the host; `simulateJoin` adds participants,
 *   `simulateLeave` takes them offline. `ResetLink`, `RemoveParticipant`,
 *   `SetParticipantRole`, `Stop` behave like the engine will.
 * - `OpenInvite` parses the link (`@/domain/invite`) and goes straight to
 *   `Joining { Ready }` with a preview from "Mock host"; a key whose secret starts with `L`
 *   is a listen link (mock convention). A room id starting with `Off` simulates a host
 *   whose app is closed (`Joining { HostOffline }`). `AcceptInvite` → `Joined` (the open
 *   project stands for the synced copy). `Leave` → `Off`. `Reconnect` → `Joined`.
 * - `PeerSignal` is `Unsupported` (no web peer endpoint in the mock).
 */

import type { Color, JoinFailure, Participant, ReplyValue, ShareCommand, ShareNotice, ShareRole, ShareState } from "@/generated";
import { INVITE_ERROR_TEXT, parseInvite, webInviteUrl } from "@/domain/invite";
import { fail } from "../documentReducer";
import type { MockHost } from "./host";

const UNIT: ReplyValue = { type: "Unit" };
const DEFAULT_ORIGIN = "https://app.ethereal.ws";
const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
/** Host colour in the mock (data, like track colours). */
export const MOCK_HOST_COLOR = 0x5cffe8;

function randomId(): string {
  let s = "";
  for (let i = 0; i < 22; i++) s += B64[Math.floor(Math.random() * 64)];
  return s;
}

export class MockShare {
  state: ShareState = { type: "Off" };
  name = "Me";
  color: Color | null = null;
  inviteOrigin = DEFAULT_ORIGIN;
  /** Last `SetPreferences` (the engine applies these; the mock only records them). */
  preferences = { resumeOnOpen: true, autoListen: true, relayOnly: false };
  private room = "";
  private keys: Record<ShareRole, string> = { Edit: "", Listen: "" };
  private nextMember = 1;

  constructor(private readonly host: MockHost) {}

  command(c: ShareCommand): ReplyValue {
    switch (c.type) {
      case "Get":
        this.emit();
        return UNIT;
      case "SetIdentity": {
        const name = c.name.trim();
        if (!name || [...name].length > 64) fail("InvalidArgument", "names are 1-64 characters");
        this.name = name;
        this.color = c.color;
        return UNIT;
      }
      case "SetServers":
        this.inviteOrigin = c.invite_origin ?? DEFAULT_ORIGIN;
        return UNIT;
      case "SetPreferences":
        this.preferences = { resumeOnOpen: c.resume_on_open, autoListen: c.auto_listen, relayOnly: c.relay_only };
        return UNIT;
      case "Start": {
        const project = this.host.project().id;
        if (this.state.type === "Joined" || this.state.type === "Joining") fail("InvalidState", "leave the session first");
        if (this.state.type === "Hosting" && this.state.project === project) return UNIT;
        this.room = randomId();
        this.keys = { Edit: `1${randomId()}`, Listen: `1L${randomId().slice(1)}` };
        this.state = {
          type: "Hosting",
          project,
          edit_link: this.link("Edit"),
          listen_link: this.link("Listen"),
          participants: [this.me("Host")],
          signal: { type: "Online" },
        };
        this.emit();
        return UNIT;
      }
      case "Stop":
        this.requireHosting();
        this.state = { type: "Off" };
        this.emit();
        return UNIT;
      case "ResetLink": {
        const s = this.requireHosting();
        this.keys[c.role] = c.role === "Listen" ? `1L${randomId().slice(1)}` : `1${randomId()}`;
        this.state = c.role === "Edit" ? { ...s, edit_link: this.link("Edit") } : { ...s, listen_link: this.link("Listen") };
        this.emit();
        return UNIT;
      }
      case "RemoveParticipant": {
        const s = this.requireHosting();
        if (!s.participants.some((p) => p.member === c.member)) fail("NotFound", `member ${c.member}`);
        this.state = { ...s, participants: s.participants.filter((p) => p.member !== c.member) };
        this.emit();
        return UNIT;
      }
      case "SetParticipantRole": {
        const s = this.requireHosting();
        if (!s.participants.some((p) => p.member === c.member)) fail("NotFound", `member ${c.member}`);
        this.state = { ...s, participants: s.participants.map((p) => (p.member === c.member ? { ...p, role: c.role } : p)) };
        this.emit();
        return UNIT;
      }
      case "OpenInvite": {
        if (this.state.type === "Hosting") fail("InvalidState", "stop sharing first");
        const inv = parseInvite(c.link);
        if (typeof inv === "string") return this.failJoin("BadLink", INVITE_ERROR_TEXT[inv]);
        if (!inv.key) return this.failJoin("BadLink", "This link has no invite key.");
        if (inv.room.startsWith("Off")) {
          this.state = { type: "Joining", stage: { type: "HostOffline", local_copy: null } };
          this.emit();
          return UNIT;
        }
        const role: ShareRole = inv.key[1] === "L" ? "Listen" : "Edit";
        this.state = {
          type: "Joining",
          stage: {
            type: "Ready",
            invite: {
              host: { name: "Mock host", color: MOCK_HOST_COLOR },
              project: this.host.project().id,
              project_name: "Shared song",
              role,
              online: [{ name: "Mock host", color: MOCK_HOST_COLOR }],
              local_copy: false,
            },
          },
        };
        this.emit();
        return UNIT;
      }
      case "AcceptInvite": {
        const s = this.state;
        if (s.type !== "Joining" || s.stage.type !== "Ready") fail("InvalidState", "no invite to accept");
        const { invite } = s.stage;
        this.state = {
          type: "Joined",
          project: invite.project,
          role: invite.role,
          participants: [this.mockHost(), this.me(invite.role)],
          link: { type: "Online" },
        };
        this.emit();
        return UNIT;
      }
      case "Leave":
        if (this.state.type === "Hosting") fail("InvalidState", "the host stops sharing instead");
        this.state = { type: "Off" };
        this.emit();
        return UNIT;
      case "Reconnect":
        this.state = {
          type: "Joined",
          project: c.project,
          role: "Edit",
          participants: [this.mockHost(), this.me("Edit")],
          link: { type: "Online" },
        };
        this.emit();
        return UNIT;
      case "Detach":
        return UNIT;
      case "PeerSignal":
        return fail("Unsupported", "the mock has no web peer endpoint");
    }
  }

  /** Someone joins this site's share (host side). Returns the member id. */
  simulateJoin(name: string, role: ShareRole, color: Color = 0xff94a6): string {
    const s = this.requireHosting();
    const member = `member-${this.nextMember++}`;
    const p: Participant = { member, site: String(100 + this.nextMember), name, color, role, online: true, you: false, last_seen_ms: null };
    this.state = { ...s, participants: [...s.participants, p] };
    this.emit();
    this.notice({ type: "ParticipantJoined", name, color });
    return member;
  }

  /** A participant goes offline (host side). */
  simulateLeave(member: string): void {
    const s = this.requireHosting();
    const p = s.participants.find((x) => x.member === member);
    if (!p) return;
    this.state = { ...s, participants: s.participants.map((x) => (x === p ? { ...x, online: false, site: null, last_seen_ms: Date.now() } : x)) };
    this.emit();
    this.notice({ type: "ParticipantLeft", name: p.name });
  }

  /** The host's app closes or comes back (joiner side). */
  simulateHostOnline(online: boolean): void {
    const s = this.state;
    if (s.type !== "Joined") return;
    this.state = { ...s, link: online ? { type: "Online" } : { type: "HostOffline", since_ms: Date.now() } };
    this.emit();
    this.notice(online ? { type: "HostBack", host_name: "Mock host" } : { type: "HostOffline", host_name: "Mock host" });
  }

  private link(role: ShareRole): string {
    return webInviteUrl({ room: this.room, key: this.keys[role], signalUrl: null }, this.inviteOrigin);
  }

  private me(role: Participant["role"]): Participant {
    return { member: role === "Host" ? null : "me", site: "1", name: this.name, color: this.color ?? 0xffb454, role, online: true, you: true, last_seen_ms: null };
  }

  private mockHost(): Participant {
    return { member: null, site: "2", name: "Mock host", color: MOCK_HOST_COLOR, role: "Host", online: true, you: false, last_seen_ms: null };
  }

  private requireHosting(): Extract<ShareState, { type: "Hosting" }> {
    if (this.state.type !== "Hosting") fail("InvalidState", "not sharing");
    return this.state;
  }

  private failJoin(reason: JoinFailure, message: string): ReplyValue {
    this.state = { type: "Joining", stage: { type: "Failed", reason, message } };
    this.emit();
    return UNIT;
  }

  private emit(): void {
    this.host.emit({ type: "Share", event: { type: "State", state: this.state } });
  }

  private notice(notice: ShareNotice): void {
    this.host.emit({ type: "Share", event: { type: "Notice", notice } });
  }
}
