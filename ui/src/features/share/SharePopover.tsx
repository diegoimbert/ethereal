// The Share popover (docs/SHARING.md §8.2): the host's links, identity, people and the
// "Stop sharing" footer; the joiner's view of the session with "Leave".
import clsx from "clsx";
import { Check, Copy, Ellipsis, Headphones, Loader2 } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { CollabCommand, Participant, Presence, ShareRole, ShareState } from "@/generated";
import { Badge, Button, IconButton, Menu, Select, Tabs, TextInput, Toggle, type MenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { HostingSection } from "@/features/collab/host";
import { listenBlocker } from "@/features/collab/listen";
import { listenTo, stopListening } from "@/features/collab/listen/agent";
import { activeHost, useListenStore } from "@/features/collab/listen/store";
import { setFollowing, useLocalPresence } from "@/features/collab/presence/local";
import { peerColor, useCollabStore, useHideOthers } from "@/features/collab/store";
import { attempt, ROLE_LABEL } from "./actions";
import { useConfirm } from "./confirmStore";
import { openJoinWithLink } from "./join/store";
import { PeerAvatar } from "./PeerAvatar";
import { NAME_MAX, nameError, PEER_COLORS, pushIdentity, useShareSettings } from "./settings";
import { lastSeenText } from "./status";
import { hostOf } from "./store";
import { shareToast } from "./toasts";

const ROLE_OPTIONS = (["Edit", "Listen"] as const).map((r) => ({ value: r, label: ROLE_LABEL[r] }));

/** The popover body for the current state. */
export function SharePopoverContent({ transport, state, close }: { transport: EngineTransport; state: ShareState; close: () => void }) {
  const name = useProjectStore((s) => s.project?.settings.name) || "this project";
  if (state.type === "Hosting") return <HostPanel transport={transport} state={state} projectName={name} close={close} />;
  if (state.type === "Joined") return <JoinerPanel transport={transport} state={state} projectName={name} close={close} />;
  // `Joining` is the join screen's (join-flow); the pill only offers to cancel.
  return (
    <div className="eth-share-pop" data-testid="share-popover">
      <header className="eth-share-pop__title">Joining a shared project…</header>
      <div className="eth-share-pop__footer">
        <span />
        <Button
          size="sm"
          onClick={() => {
            close();
            void attempt(transport, cmd("Share", { type: "Leave" }), "Couldn't cancel");
          }}
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}

// ─── host ───────────────────────────────────────────────────────────────────────────────

type Hosting = Extract<ShareState, { type: "Hosting" }>;
type Joined = Extract<ShareState, { type: "Joined" }>;

function HostPanel({ transport, state, projectName, close }: { transport: EngineTransport; state: Hosting; projectName: string; close: () => void }) {
  const confirm = useConfirm((s) => s.ask);
  const stop = () =>
    confirm({
      title: `Stop sharing “${projectName}”?`,
      body: stopSharingBody(state.participants),
      action: "Stop sharing",
      danger: true,
      run: () => void attempt(transport, cmd("Share", { type: "Stop" }), "Couldn't stop sharing"),
    });
  const reset = (role: ShareRole) =>
    confirm({
      title: `Reset the ${role === "Edit" ? "edit" : "listen"} link?`,
      body: "The current link stops working. People already in keep access.",
      action: "Reset link",
      run: () =>
        void attempt(transport, cmd("Share", { type: "ResetLink", role }), "Couldn't reset the link").then(
          (ok) => ok && shareToast("Link reset", "The old one no longer works."),
        ),
    });
  const menu: MenuEntry[] = [
    { id: "reset-edit", label: "Reset edit link", onSelect: () => reset("Edit") },
    { id: "reset-listen", label: "Reset listen link", onSelect: () => reset("Listen") },
    { separator: true, id: "sep-join" },
    // Joining someone else's project (docs/SHARING.md §5): paste their link.
    {
      id: "join",
      label: "Join with a link…",
      onSelect: () => {
        close();
        openJoinWithLink();
      },
    },
  ];
  return (
    <div className="eth-share-pop" data-testid="share-popover">
      <header className="eth-share-pop__title">Share “{projectName}”</header>
      <LinkRow state={state} />
      <IdentitySection transport={transport} />
      <PeopleSection transport={transport} participants={state.participants} hosting />
      <details className="eth-share-pop__more">
        <summary>Listening and visibility</summary>
        <HostingSection transport={transport} />
        <HideOthersToggle />
      </details>
      <div className="eth-share-pop__footer">
        <Menu
          aria-label="More sharing actions"
          placement="top-start"
          items={menu}
          trigger={(p) => <IconButton {...p} size="sm" tone="ghost" label="More sharing actions" icon={<Ellipsis />} />}
        />
        <Button
          size="sm"
          tone="danger"
          onClick={() => {
            close();
            stop();
          }}
        >
          Stop sharing
        </Button>
      </div>
    </div>
  );
}

function stopSharingBody(participants: ReadonlyArray<Participant>): string {
  const others = participants.filter((p) => !p.you).map((p) => p.name || "Someone");
  const who = others.length === 0 ? "" : others.length === 1 ? `${others[0]} keeps` : `${others.slice(0, -1).join(", ")} and ${others.at(-1)} keep`;
  return who ? `${who} an offline copy. Links stop working.` : "Links stop working.";
}

/** How long the Copy button reads "Copied". */
const COPIED_MS = 2000;

/** "Can edit" | "Can listen", the link and a Copy button (§8.2.1). */
function LinkRow({ state }: { state: Hosting }) {
  const [role, setRole] = useState<ShareRole>("Edit");
  // The link last copied: "Copied" shows for it only (a reset link reads "Copy" again).
  const [copiedLink, setCopiedLink] = useState<string | null>(null);
  const row = useRef<HTMLDivElement>(null);
  const link = role === "Edit" ? state.edit_link : state.listen_link;
  const copied = link !== null && copiedLink === link;
  useEffect(() => {
    if (!copiedLink) return;
    const t = setTimeout(() => setCopiedLink(null), COPIED_MS);
    return () => clearTimeout(t);
  }, [copiedLink]);
  const copy = async () => {
    if (!link) return;
    try {
      await navigator.clipboard.writeText(link);
    } catch {
      // No clipboard permission: select it, so ⌘C works.
      row.current?.querySelector("input")?.select();
      return;
    }
    setCopiedLink(link);
    shareToast("Link copied", role === "Edit" ? "Anyone with this link can edit." : "Anyone with this link can listen.");
  };
  const offline = state.signal.type === "Offline";
  return (
    <section className="eth-share-pop__section" aria-label="Invite link">
      <Tabs
        label="Link"
        value={role}
        onChange={setRole}
        items={[
          { id: "Edit", label: ROLE_LABEL.Edit },
          { id: "Listen", label: ROLE_LABEL.Listen },
        ]}
      />
      {link ? (
        <div className="eth-share-pop__link" ref={row}>
          <TextInput
            size="sm"
            readOnly
            value={link}
            aria-label={role === "Edit" ? "Edit link" : "Listen link"}
            onFocus={(e) => e.currentTarget.select()}
          />
          <Button size="sm" tone="accent" onClick={() => void copy()} data-testid="share-copy">
            {copied ? <Check className="eth-share-pop__icon" aria-hidden /> : <Copy className="eth-share-pop__icon" aria-hidden />}
            {copied ? "Copied" : "Copy"}
          </Button>
        </div>
      ) : offline ? null : (
        <p className="eth-share-pop__status" role="status">
          <Loader2 className="eth-share-pop__icon eth-share-spin" aria-hidden />
          Getting a link…
        </p>
      )}
      {offline ? (
        <p className="eth-share-pop__warn" role="alert">
          Can't reach the sharing service. People already here stay connected.
        </p>
      ) : (
        <p className="eth-share-pop__hint">
          {role === "Edit" ? "Anyone with this link can edit." : "Anyone with this link can listen and chat, but not edit."}
        </p>
      )}
    </section>
  );
}

/** "You appear as": name and colour (§8.2.2). Expanded until a name is chosen. */
export function IdentitySection({ transport, alwaysOpen }: { transport: EngineTransport; alwaysOpen?: boolean }) {
  const name = useShareSettings((s) => s.name);
  const color = useShareSettings((s) => s.color);
  const [editing, setEditing] = useState(() => alwaysOpen || !!nameError(name));
  // The field follows the saved name when it changes elsewhere (Settings vs popover).
  const [edit, setEdit] = useState({ base: name, value: name });
  if (edit.base !== name) setEdit({ base: name, value: name });
  const draft = edit.value;
  const setDraft = (value: string) => setEdit({ base: name, value });
  const problem = draft !== name ? nameError(draft) : null;
  const commit = () => {
    if (nameError(draft) || draft.trim() === name) return;
    useShareSettings.getState().update({ name: draft.trim() });
    pushIdentity(transport);
  };
  const pick = (c: number) => {
    useShareSettings.getState().update({ color: c });
    pushIdentity(transport);
  };
  const shown = color ?? PEER_COLORS[0]!;
  if (!editing) {
    return (
      <section className="eth-share-pop__section eth-share-pop__me" aria-label="You appear as">
        <span className="eth-share-pop__label">You appear as</span>
        <PeerAvatar name={name} color={shown} />
        <span className="eth-share-pop__name">{name}</span>
        <Button size="sm" tone="ghost" onClick={() => setEditing(true)}>
          Change
        </Button>
      </section>
    );
  }
  return (
    <section className="eth-share-pop__section" aria-label="You appear as">
      <span className="eth-share-pop__label">You appear as</span>
      <div className="eth-share-pop__identity">
        <PeerAvatar name={draft || "?"} color={shown} />
        <TextInput
          size="sm"
          aria-label="Your name"
          placeholder="Your name"
          value={draft}
          maxLength={NAME_MAX + 8}
          invalid={!!problem}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              commit();
              if (!alwaysOpen && !nameError(draft)) setEditing(false);
            }
          }}
        />
      </div>
      {problem && (
        <p className="eth-share-pop__warn" role="alert">
          {problem}
        </p>
      )}
      <div className="eth-share-swatches" role="group" aria-label="Your colour">
        {PEER_COLORS.map((c) => (
          <button
            key={c}
            type="button"
            className="eth-share-swatch"
            style={{ "--eth-share-peer": peerColor(c) } as CSSProperties}
            aria-pressed={c === shown}
            aria-label={`Colour ${PEER_COLORS.indexOf(c) + 1}`}
            onClick={() => pick(c)}
          />
        ))}
      </div>
      <p className="eth-share-pop__hint">Others see this name and colour. The host may change a colour that is taken.</p>
    </section>
  );
}

// ─── people ─────────────────────────────────────────────────────────────────────────────

/** The participants (§8.2.3): avatar, name, role, online dot; row actions. */
function PeopleSection({ transport, participants, hosting }: { transport: EngineTransport; participants: ReadonlyArray<Participant>; hosting: boolean }) {
  const online = participants.filter((p) => p.online).length;
  return (
    <section className="eth-share-pop__section" aria-label="People">
      <span className="eth-share-pop__label">
        People <span className="eth-share-pop__count">{online} online</span>
      </span>
      <ul className="eth-share-people" aria-label="People">
        {participants.map((p) => (
          <PersonRow key={p.member ?? `host-${p.site ?? p.name}`} transport={transport} person={p} hosting={hosting} />
        ))}
      </ul>
    </section>
  );
}

function PersonRow({ transport, person: p, hosting }: { transport: EngineTransport; person: Participant; hosting: boolean }) {
  const peers = useCollabStore((s) => s.peers);
  const presence = p.site !== null ? peers.find((x) => x.site === p.site) : undefined;
  const following = useLocalPresence((s) => p.site !== null && s.following === p.site);
  const listening = useListenStore((s) => (p.site !== null ? activeHost(s.listening) === p.site : false));
  const name = p.name || "Anonymous";
  const member = p.member;

  const items: MenuEntry[] = [];
  if (!p.you && p.online && presence) {
    items.push({ id: "follow", label: following ? "Stop following" : `Follow ${name}`, onSelect: () => setFollowing(following ? null : presence.site) });
    items.push(listenEntry(transport, presence, listening));
  }
  if (hosting && member && !p.you) {
    if (items.length) items.push({ separator: true, id: "sep" });
    items.push({
      id: "remove",
      label: "Remove from project",
      danger: true,
      onSelect: () =>
        void attempt(transport, cmd("Share", { type: "RemoveParticipant", member }), `Couldn't remove ${name}`).then(
          (ok) => ok && shareToast(`${name} was removed`, "They can come back only with a current link."),
        ),
    });
  }

  return (
    <li className={clsx("eth-share-person", !p.online && "eth-share-person--offline")} data-testid="share-person" data-member={member ?? "host"}>
      <PeerAvatar name={name} color={p.color} offline={!p.online} />
      <span className="eth-share-person__name">
        {name}
        {p.you && <span className="eth-share-person__you"> (you)</span>}
        <span className={clsx("eth-share-person__dot", p.online && "eth-share-person__dot--on")} aria-label={p.online ? "online" : "offline"} role="img" />
        {!p.online && <span className="eth-share-person__seen">{lastSeenText(p.last_seen_ms)}</span>}
        {listening && <Headphones className="eth-share-pop__icon" aria-label={`Listening to ${name}`} />}
      </span>
      {p.role === "Host" ? (
        <Badge tone="accent">Host</Badge>
      ) : hosting && member && !p.you ? (
        <Select
          size="sm"
          aria-label={`${name}'s role`}
          value={p.role}
          options={ROLE_OPTIONS}
          onChange={(role) => void attempt(transport, cmd("Share", { type: "SetParticipantRole", member, role }), `Couldn't change ${name}'s role`)}
        />
      ) : (
        <Badge>{ROLE_LABEL[p.role]}</Badge>
      )}
      {items.length > 0 ? (
        <Menu
          aria-label={`Actions for ${name}`}
          placement="bottom-end"
          items={items}
          trigger={(t) => <IconButton {...t} size="sm" tone="ghost" label={`Actions for ${name}`} icon={<Ellipsis />} />}
        />
      ) : (
        <span className="eth-share-person__spacer" />
      )}
    </li>
  );
}

const collabSend = (transport: EngineTransport) => (c: CollabCommand) => transport.send(cmd("Collab", c));

function listenEntry(transport: EngineTransport, presence: Presence, listening: boolean): MenuEntry {
  const send = collabSend(transport);
  const name = presence.name || "Anonymous";
  if (listening) return { id: "listen", label: "Stop listening", onSelect: () => void stopListening(send) };
  const blocker = listenBlocker(presence);
  return {
    id: "listen",
    label: blocker ? `Listen on ${name} (${blocker})` : `Listen on ${name}`,
    disabled: blocker !== null,
    onSelect: () => void listenTo(send, presence.site, presence.name),
  };
}

/** "Hide users and notes" (docs/COLLAB.md §12.4), a local preference. */
function HideOthersToggle() {
  const hide = useHideOthers();
  const set = useCollabStore((s) => s.setHideOthers);
  return (
    <div className="eth-share-pop__toggle">
      <Toggle size="sm" checked={hide} onChange={set} label="Hide users and notes" />
      <p className="eth-share-pop__hint">Hides the others' pointers, playheads, selections and pinned notes on your screen only.</p>
    </div>
  );
}

// ─── joiner ─────────────────────────────────────────────────────────────────────────────

function JoinerPanel({ transport, state, projectName, close }: { transport: EngineTransport; state: Joined; projectName: string; close: () => void }) {
  const host = hostOf(state);
  const hostName = host?.name || "the host";
  return (
    <div className="eth-share-pop" data-testid="share-popover">
      <header className="eth-share-pop__title">“{projectName}”</header>
      <p className="eth-share-pop__hint">
        Shared by {hostName} · {state.role === "Listen" ? "you can listen and chat" : "you can edit"}
      </p>
      {state.role === "Listen" && host && <ListenToHost transport={transport} host={host} />}
      <PeopleSection transport={transport} participants={state.participants} hosting={false} />
      <details className="eth-share-pop__more">
        <summary>Visibility</summary>
        <HideOthersToggle />
      </details>
      <div className="eth-share-pop__footer">
        <span className="eth-share-pop__hint">Leaving keeps your copy on this computer.</span>
        <Button
          size="sm"
          tone="danger"
          onClick={() => {
            close();
            void attempt(transport, cmd("Share", { type: "Leave" }), "Couldn't leave").then(
              (ok) => ok && shareToast(`You left ${projectName}`, "Your copy stays in your projects."),
            );
          }}
        >
          Leave
        </Button>
      </div>
    </div>
  );
}

/** Listen joiners: "Listening to Diego" with Stop listening / Listen again (§8.2). */
function ListenToHost({ transport, host }: { transport: EngineTransport; host: Participant }) {
  const listening = useListenStore((s) => s.listening);
  const send = collabSend(transport);
  const on = host.site !== null && activeHost(listening) === host.site;
  const name = host.name || "the host";
  return (
    <section className="eth-share-pop__section eth-share-pop__listen" aria-label="Listening">
      <Headphones className="eth-share-pop__icon" aria-hidden />
      <span className="eth-share-pop__name">{on ? (listening.type === "Connecting" ? `Connecting to ${name}…` : `Listening to ${name}`) : `Not listening to ${name}`}</span>
      {on ? (
        <Button size="sm" onClick={() => void stopListening(send)}>
          Stop listening
        </Button>
      ) : (
        <Button size="sm" disabled={host.site === null || !host.online} onClick={() => host.site !== null && void listenTo(send, host.site, host.name)}>
          Listen again
        </Button>
      )}
    </section>
  );
}
