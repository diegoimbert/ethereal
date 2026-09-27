// Listen-on-peer UX (docs/COLLAB.md §9): the dialog button and the status badge in the
// presence bar.
import "./listen.css";
import type { Presence, SiteId } from "@/generated";
import { Button, IconButton } from "@/kit";
import type { EngineTransport } from "@/transport";
import { listenTo, stopListening } from "./agent";
import { listenBlocker, peerName, send } from "./menu";
import { activeHost, useListenStore } from "./store";

/** Dialog member row action: Listen / Stop listening (disabled with its reason). */
export function ListenButton({ transport, peer }: { transport: EngineTransport; peer: Presence }) {
  const listening = useListenStore((s) => s.listening);
  if (activeHost(listening) === peer.site) {
    return (
      <Button size="sm" className="eth-listen__action" onClick={() => void stopListening(send(transport))}>
        Stop listening
      </Button>
    );
  }
  const blocker = listenBlocker(peer);
  return (
    <>
      {blocker && <span className="eth-listen__reason">{blocker}</span>}
      <Button
        size="sm"
        className={blocker ? undefined : "eth-listen__action"}
        disabled={blocker !== null}
        title={blocker ? `Unavailable: ${blocker}` : `Hear ${peerName(peer)}'s engine, with a shared playhead`}
        aria-label={`Listen on ${peerName(peer)}'s computer`}
        onClick={() => void listenTo(send(transport), peer.site, peer.name)}
      >
        Listen
      </Button>
    </>
  );
}

function Headphones() {
  return (
    <svg className="eth-listen__icon" viewBox="0 0 16 16" aria-hidden="true">
      <path d="M1.5 12V8.5a6.5 6.5 0 0 1 13 0V12h-1.5V8.5a5 5 0 0 0-10 0V12zM1.5 10h3v5h-3zM11.5 10h3v5h-3z" />
    </svg>
  );
}

/** "Listening to Diego" / "Connecting to Diego…" / why it ended, in the presence bar. */
export function ListenBadge({ transport, peers }: { transport: EngineTransport; peers: Presence[] }) {
  const listening = useListenStore((s) => s.listening);
  const error = useListenStore((s) => s.error);
  const countIn = useListenStore((s) => s.countIn);
  const hostName = useListenStore((s) => s.hostName);
  // A host that left is gone from `peers`: its name was kept when the listening started.
  const nameOf = (site: SiteId) => peerName(peers.find((p) => p.site === site) ?? { name: hostName?.[0] === site ? hostName[1] : "" });
  const dismiss = () =>
    useListenStore.setState({
      error: null,
      ...(listening.type === "Ended" ? { listening: { type: "Off" } } : {}),
    });

  // Floats under the presence bar (the top bar has no width to spare), ellipsized; the full
  // text is also the tooltip and the accessible name.
  if (listening.type === "Connecting" || listening.type === "Listening") {
    const who = nameOf(listening.host);
    const full = listening.type === "Connecting" ? `Connecting to ${who}…` : countIn ? `Listening to ${who} · count-in` : `Listening to ${who}`;

    return (
      <span className="eth-listen" data-testid="listen-status" data-state={listening.type} role="status" aria-label={full} title={full}>
        <Headphones />
        <span className="eth-listen__text" aria-hidden="true">
          {full}
        </span>
        <IconButton size="sm" tone="ghost" label="Stop listening" icon="×" onClick={() => void stopListening(send(transport))} />
      </span>
    );
  }
  const message = listening.type === "Ended" ? `Stopped listening to ${nameOf(listening.host)}: ${listening.reason}` : error;
  if (!message) return null;
  return (
    <span
      className="eth-listen eth-listen--ended"
      data-testid="listen-status"
      data-state={listening.type}
      role="alert"
      aria-label={message}
      title={message}
    >
      <Headphones />
      <span className="eth-listen__text" aria-hidden="true">
        {message}
      </span>
      <IconButton size="sm" tone="ghost" label="Dismiss" icon="×" onClick={dismiss} />
    </span>
  );
}
