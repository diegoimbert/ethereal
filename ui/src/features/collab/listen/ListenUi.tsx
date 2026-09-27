// Listen-on-peer UX (docs/COLLAB.md §9): the dialog button and the status badge in the
// presence bar.
import "./listen.css";
import type { Presence, SiteId } from "@/generated";
import { Button } from "@/kit";
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
    <Button
      size="sm"
      className="eth-listen__action"
      disabled={blocker !== null}
      title={blocker ? `Unavailable: ${blocker}` : `Hear ${peerName(peer)}'s engine, with a shared playhead`}
      aria-label={`Listen on ${peerName(peer)}'s computer`}
      onClick={() => void listenTo(send(transport), peer.site, peer.name)}
    >
      Listen
    </Button>
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

  if (listening.type === "Connecting" || listening.type === "Listening") {
    const who = nameOf(listening.host);
    return (
      <span className="eth-listen" data-testid="listen-status" data-state={listening.type} role="status">
        <span className="eth-listen__text">
          {listening.type === "Connecting" ? `Connecting to ${who}…` : countIn ? `Listening to ${who} · count-in` : `Listening to ${who}`}
        </span>
        <Button size="sm" onClick={() => void stopListening(send(transport))}>
          Stop
        </Button>
      </span>
    );
  }
  const message = listening.type === "Ended" ? `Stopped listening to ${nameOf(listening.host)}: ${listening.reason}` : error;
  if (!message) return null;
  return (
    <span className="eth-listen eth-listen--ended" data-testid="listen-status" data-state={listening.type} role="alert">
      <span className="eth-listen__text">{message}</span>
      <Button size="sm" aria-label="Dismiss" onClick={dismiss}>
        OK
      </Button>
    </span>
  );
}
