import "./collab.css";
import { useContext, useEffect, useMemo, useState, type FormEvent } from "react";
import type { Presence, PresenceState, SiteId } from "@/generated";
import { Button, Dialog, openContextMenu, TextInput, Toggle } from "@/kit";
import { useSelectionStore } from "@/state/selection";
import { itemSelection } from "@/timeline/selection";
import { cmd, TransportContext, type EngineTransport } from "@/transport";
import { useEngineNotifications } from "@/features/notifications";
import { HostingBadge, HostingSection, useHosting } from "./host";
import { relayError, sessionNameError, tokenStorage } from "./joinFields";
import { ListenBadge, ListenButton, listenMenuItems, useListenAgent } from "./listen";
import { avatarStyle } from "./presence/avatar";
import { nameOf, peerSummary, presenceV2Fields, setFollowing, useLocalPresence } from "./presence/local";
import { ChatToasts } from "./social";
import { highlightCss, initials, useCollabStore, useHideOthers } from "./store";

/** Remembered join fields (the token is kept apart: `tokenStorage`). */
const FIELDS_KEY = "eth-collab-join";

interface JoinFields {
  server: string;
  session: string;
  name: string;
}

function loadFields(): JoinFields {
  try {
    const v = JSON.parse(localStorage.getItem(FIELDS_KEY) ?? "{}") as Partial<JoinFields>;
    return { server: v.server ?? "", session: v.session ?? "", name: v.name ?? "" };
  } catch {
    return { server: "", session: "", name: "" };
  }
}

function saveFields(f: JoinFields) {
  try {
    localStorage.setItem(FIELDS_KEY, JSON.stringify(f));
  } catch {
    // storage unavailable: nothing to remember
  }
}

/** The session field accepts a few characters past the limit, so the inline error can show. */
const SESSION_INPUT_MAX = 80;

const describe = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** This site's presence from the app selection (selected track, clips, notes). */
function currentPresence(): PresenceState {
  const track = useSelectionStore.getState().selectedTrack;
  const sel = itemSelection.getState().selected;
  return {
    cursor: null,
    selected_tracks: track ? [track] : [],
    selected_clips: [...sel.clip],
    selected_notes: [...sel.note],
    selected_devices: [],
    view: null,
    // presence v2 (docs/COLLAB.md §8): activity, viewport, following.
    ...presenceV2Fields(),
  };
}

/** Publish the local selection as presence while in a session (the engine throttles). */
function usePublishPresence(transport: EngineTransport, active: boolean) {
  useEffect(() => {
    if (!active) return;
    let last = "";
    const publish = () => {
      const presence = currentPresence();
      const key = JSON.stringify(presence);
      if (key === last) return;
      last = key;
      void transport.send(cmd("Collab", { type: "SetPresence", presence })).catch(() => undefined);
    };
    publish();
    const offTrack = useSelectionStore.subscribe(publish);
    const offItems = itemSelection.subscribe(publish);
    const offV2 = useLocalPresence.subscribe(publish);
    return () => {
      offTrack();
      offItems();
      offV2();
    };
  }, [active, transport]);
}

/**
 * A peer's avatar: a click follows its view (click again, Escape, or a local scroll/zoom
 * stops); right-click offers the same. A headphones badge shows when it listens to someone
 * (docs/COLLAB.md §8.4).
 */
function PeerChip({ peer, peers, me, transport }: { peer: Presence; peers: Presence[]; me: SiteId | null; transport: EngineTransport }) {
  const following = useLocalPresence((s) => s.following === peer.site);
  const name = peer.name || "Anonymous";
  const toggle = () => setFollowing(following ? null : peer.site);
  const listening = peer.state.listening_to;
  const hint = following ? "Click or press Escape to stop following" : `Click to follow ${name}'s view`;
  return (
    <button
      type="button"
      className="eth-collab__avatar eth-collab__chip"
      style={avatarStyle(peer.color)}
      title={`${peerSummary(peer, peers, me)}\n${hint}`}
      aria-label={following ? `Stop following ${name}` : `Follow ${name}`}
      aria-pressed={following}
      data-peer={peer.name}
      data-following={following || undefined}
      data-followed={me !== null && peer.state.following === me ? "you" : undefined}
      onClick={toggle}
      onContextMenu={(e) =>
        openContextMenu(e, [{ label: following ? "Stop following" : `Follow ${name}`, onSelect: toggle }, "separator", ...listenMenuItems(transport, peer)])
      }
    >
      {initials(peer.name)}
      {listening && (
        <span className="eth-collab__badge" data-testid="listening-badge" title={`${name} is listening to ${nameOf(listening, peers, me)}`}>
          <svg viewBox="0 0 16 16" aria-hidden>
            <path d="M1.5 12V8.5a6.5 6.5 0 0 1 13 0V12h-1.5V8.5a5 5 0 0 0-10 0V12zM1.5 10h3v5h-3zM11.5 10h3v5h-3z" />
          </svg>
        </span>
      )}
    </button>
  );
}

/** Outlines of the peers' selections, in their colors. */
function PeerHighlights() {
  const peers = useCollabStore((s) => s.peers);
  const hide = useHideOthers();
  const css = hide ? "" : highlightCss(peers);
  return css ? <style data-testid="collab-highlights">{css}</style> : null;
}

/**
 * "Hide users and notes" (docs/COLLAB.md §12.4): a local preference, never sent. Peers' edits,
 * the chat and these avatars stay.
 */
function HideOthersToggle() {
  const hide = useHideOthers();
  const set = useCollabStore((s) => s.setHideOthers);
  return (
    <div className="eth-collab__hide" data-testid="collab-hide-others">
      <Toggle size="sm" checked={hide} onChange={set} label="Hide users and notes" />
      <p className="eth-collab__hint">Hides the others' pointers, playheads, selections and pinned notes on your screen only.</p>
    </div>
  );
}

/**
 * Collaboration (top bar `data-slot="collab"`): join/leave a session on a relay, and the
 * other participants as colored avatars; their selections are outlined in their color.
 */
export function PresenceBar() {
  const ctx = useContext(TransportContext);
  // Outside a TransportProvider (shell tests) the slot stays empty.
  if (!ctx) return <div className="eth-collab" data-feature="collab" />;
  return <PresenceBarWith transport={ctx.transport} />;
}

function PresenceBarWith({ transport }: { transport: EngineTransport }) {
  const status = useCollabStore((s) => s.status);
  const peers = useCollabStore((s) => s.peers);
  const onEvent = useCollabStore((s) => s.onEvent);
  const open = useCollabStore((s) => s.dialogOpen);
  const setOpen = useCollabStore((s) => s.setDialogOpen);
  const [fields, setFields] = useState(loadFields);
  const [token, setToken] = useState("");
  const [remember, setRemember] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Inline errors show once a join was attempted (the session one also while typing). */
  const [tried, setTried] = useState(false);
  const tokens = useMemo(() => tokenStorage(transport), [transport]);

  useEffect(() => transport.onEvent(onEvent), [transport, onEvent]);
  // base-114: the engine's notifications as toasts (shared stack, see ChatToasts).
  useEngineNotifications(transport);
  // The remembered token (never logged).
  useEffect(() => {
    let active = true;
    void tokens.load().then((t) => {
      if (active && t) setToken((cur) => cur || t);
    });
    return () => {
      active = false;
    };
  }, [tokens]);
  useListenAgent(transport);
  // Current state after (re)connecting to an engine (it may already be in a session).
  useEffect(() => {
    useCollabStore.getState().reset();
    void transport.send(cmd("Collab", { type: "Get" })).catch(() => undefined);
  }, [transport]);
  usePublishPresence(transport, status.type === "Online");
  useHosting(transport, status.type === "Online" ? status.session : null);

  const inSession = status.type !== "Offline";
  const join = async (e?: FormEvent) => {
    e?.preventDefault();
    const f = { server: fields.server.trim(), session: fields.session.trim(), name: fields.name.trim() };
    setTried(true);
    if (relayError(f.server) || sessionNameError(f.session)) return;
    setBusy(true);
    setError(null);
    try {
      await transport.send(cmd("Collab", { type: "Join", server: f.server, session: f.session, token: token || null, name: f.name }));
      saveFields(f);
      void tokens.save(remember && token ? token : null);
      setOpen(false);
    } catch (err) {
      setError(describe(err));
    } finally {
      setBusy(false);
    }
  };
  const leave = () => {
    void transport.send(cmd("Collab", { type: "Leave" })).catch(() => undefined);
    setOpen(false);
  };

  const label =
    status.type === "Online" ? `● ${status.session}` : status.type === "Connecting" ? `Connecting to ${status.session}…` : "Collab";
  const sessionField = fields.session.trim();
  const sessionProblem = tried || sessionField ? sessionNameError(sessionField) : null;
  const relayProblem = tried ? relayError(fields.server.trim()) : null;
  return (
    <div className="eth-collab" data-feature="collab" data-status={status.type}>
      <Button
        size="sm"
        active={status.type === "Online"}
        aria-label="Collaboration"
        aria-haspopup="dialog"
        title={inSession ? "Collaboration session" : "Join or start a collaboration session"}
        data-testid="collab-button"
        onClick={() => setOpen(true)}
      >
        {label}
      </Button>
      <HostingBadge />
      {peers.length > 0 && (
        <span className="eth-collab__peers" aria-label="Participants" data-testid="collab-peers">
          {peers.map((p) => (
            <PeerChip key={p.site} peer={p} peers={peers} me={status.type === "Online" ? status.site : null} transport={transport} />
          ))}
        </span>
      )}
      <ListenBadge transport={transport} peers={peers} />
      <PeerHighlights />
      <ChatToasts />
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        title="Collaboration"
        footer={
          inSession ? (
            <>
              <Button onClick={() => setOpen(false)}>Close</Button>
              <Button tone="danger" onClick={leave}>
                Leave session
              </Button>
            </>
          ) : (
            <>
              <Button onClick={() => setOpen(false)}>Cancel</Button>
              <Button tone="accent" disabled={busy} onClick={() => void join()}>
                {busy ? "Joining…" : "Join"}
              </Button>
            </>
          )
        }
      >
        {inSession ? (
          <div className="eth-collab__form" data-testid="collab-session">
            <p>
              {status.type === "Online" ? "In session" : "Connecting to"} <strong>{status.session}</strong>
            </p>
            <ul className="eth-collab__list" aria-label="Participants">
              <li className="eth-collab__member">You ({fields.name || "this device"})</li>
              {peers.map((p) => (
                <li key={p.site} className="eth-collab__member">
                  <span className="eth-collab__avatar" style={avatarStyle(p.color)}>
                    {initials(p.name)}
                  </span>
                  {p.name || "Anonymous"}
                  <ListenButton transport={transport} peer={p} />
                </li>
              ))}
            </ul>
            <HostingSection transport={transport} />
            <HideOthersToggle />
            <p className="eth-collab__hint">Everyone edits the same project; playback, solo, loop and metronome stay yours.</p>
          </div>
        ) : (
          <form className="eth-collab__form" onSubmit={(e) => void join(e)}>
            <label className="eth-collab__field">
              <span>Relay</span>
              <TextInput
                aria-label="Relay address"
                placeholder="ws://host:port"
                value={fields.server}
                autoFocus
                invalid={!!relayProblem}
                aria-describedby={relayProblem ? "eth-collab-relay-error" : undefined}
                onChange={(e) => setFields({ ...fields, server: e.target.value })}
              />
            </label>
            {relayProblem && (
              <p className="eth-collab__error eth-collab__field-error" id="eth-collab-relay-error" role="alert">
                {relayProblem}
              </p>
            )}
            <label className="eth-collab__field">
              <span>Session</span>
              <TextInput
                aria-label="Session"
                placeholder="my-song"
                value={fields.session}
                maxLength={SESSION_INPUT_MAX}
                invalid={!!sessionProblem}
                aria-describedby={sessionProblem ? "eth-collab-session-error" : undefined}
                onChange={(e) => setFields({ ...fields, session: e.target.value })}
              />
            </label>
            {sessionProblem && (
              <p className="eth-collab__error eth-collab__field-error" id="eth-collab-session-error" role="alert">
                {sessionProblem}
              </p>
            )}
            <label className="eth-collab__field">
              <span>Your name</span>
              <TextInput aria-label="Your name" value={fields.name} onChange={(e) => setFields({ ...fields, name: e.target.value })} />
            </label>
            <label className="eth-collab__field">
              <span>Token</span>
              <TextInput aria-label="Token" type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
            </label>
            <div className="eth-collab__remember">
              <Toggle size="sm" checked={remember} onChange={setRemember} label="Remember" />
              <p className="eth-collab__hint">
                {tokens.where === "app"
                  ? "Kept in Ethereal's app data folder on this computer."
                  : "Kept in this browser's local storage: anyone using this browser profile can read it."}
              </p>
            </div>
            <p className="eth-collab__hint">
              The first participant shares the open project; others get a copy. Start a relay with <code>ether-collab-relay</code>, which prints its
              address and token.
            </p>
            {error && (
              <p className="eth-collab__error" role="alert">
                {error}
              </p>
            )}
            <button type="submit" hidden />
          </form>
        )}
      </Dialog>
    </div>
  );
}
