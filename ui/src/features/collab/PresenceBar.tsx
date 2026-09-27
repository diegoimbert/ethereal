import "./collab.css";
import { useContext, useEffect, useState, type CSSProperties, type FormEvent } from "react";
import type { PresenceState } from "@/generated";
import { Button, Dialog, TextInput } from "@/kit";
import { useSelectionStore } from "@/state/selection";
import { itemSelection } from "@/timeline/selection";
import { cmd, TransportContext, type EngineTransport } from "@/transport";
import { HostingBadge, HostingSection, useHosting } from "./host";
import { highlightCss, initials, peerColor, useCollabStore } from "./store";

/** Remembered join fields (never the token). */
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
    return () => {
      offTrack();
      offItems();
    };
  }, [active, transport]);
}

/** Outlines of the peers' selections, in their colors. */
function PeerHighlights() {
  const peers = useCollabStore((s) => s.peers);
  const css = highlightCss(peers);
  return css ? <style data-testid="collab-highlights">{css}</style> : null;
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
  const [open, setOpen] = useState(false);
  const [fields, setFields] = useState(loadFields);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => transport.onEvent(onEvent), [transport, onEvent]);
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
    if (!/^wss?:\/\//.test(f.server)) {
      setError("Enter the relay address, such as ws://studio.local:9003");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await transport.send(cmd("Collab", { type: "Join", server: f.server, session: f.session, token: token || null, name: f.name }));
      saveFields(f);
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
            <span
              key={p.site}
              className="eth-collab__avatar"
              style={{ "--eth-collab-peer": peerColor(p.color) } as CSSProperties}
              title={p.name || "Anonymous"}
              data-peer={p.name}
            >
              {initials(p.name)}
            </span>
          ))}
        </span>
      )}
      <PeerHighlights />
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
                  <span className="eth-collab__avatar" style={{ "--eth-collab-peer": peerColor(p.color) } as CSSProperties}>
                    {initials(p.name)}
                  </span>
                  {p.name || "Anonymous"}
                </li>
              ))}
            </ul>
            <HostingSection transport={transport} />
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
                onChange={(e) => setFields({ ...fields, server: e.target.value })}
              />
            </label>
            <label className="eth-collab__field">
              <span>Session</span>
              <TextInput aria-label="Session" placeholder="my-song" value={fields.session} onChange={(e) => setFields({ ...fields, session: e.target.value })} />
            </label>
            <label className="eth-collab__field">
              <span>Your name</span>
              <TextInput aria-label="Your name" value={fields.name} onChange={(e) => setFields({ ...fields, name: e.target.value })} />
            </label>
            <label className="eth-collab__field">
              <span>Token</span>
              <TextInput aria-label="Token" type="password" autoComplete="off" value={token} onChange={(e) => setToken(e.target.value)} />
            </label>
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
