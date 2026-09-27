// Hosting UI ("listen on <peer>", host side; docs/COLLAB.md §9.3):
// - `HostingSection`: the toggles and the listener list (collab dialog).
// - `HostingBadge`: "N listening" next to the collab button.
import "./host.css";
import { Badge, Toggle } from "@/kit";
import type { EngineTransport } from "@/transport";
import { useCollabStore } from "../store";
import { linkKey } from "./sender";
import { useHostStore } from "./store";

/** Display names of this site's listeners, with their link state. A native (engine) link's
 * state is not reported to the UI: that listener is "listening" once its presence says it
 * listens to this site (set when its media flows), "connecting" before. */
function useListenerRows() {
  const listeners = useHostStore((s) => s.listeners);
  const links = useHostStore((s) => s.links);
  const peers = useCollabStore((s) => s.peers);
  const status = useCollabStore((s) => s.status);
  const me = status.type === "Online" ? status.site : null;
  return listeners.map((l) => {
    const key = linkKey(l.site, l.stream);
    const peer = peers.find((p) => p.site === l.site);
    const name = peer?.name || "Anonymous";
    const engineState = me !== null && peer?.state.listening_to === me ? "connected" : "connecting";
    const state = l.endpoint === "Engine" ? engineState : (links[key] ?? "connecting");
    return { key, name, state };
  });
}

/** Hosting toggles and the listener list (collab dialog, in a session). */
export function HostingSection({ transport }: { transport: EngineTransport }) {
  const allow = useHostStore((s) => s.allow);
  const remoteTransport = useHostStore((s) => s.remoteTransport);
  const uiSender = useHostStore((s) => s.uiSender);
  const rows = useListenerRows();
  return (
    <section className="eth-collab-host" aria-label="Hosting" data-testid="collab-hosting">
      <Toggle size="sm" checked={allow} onChange={(v) => useHostStore.getState().setAllow(v)} label="Let others listen to my computer" />
      <Toggle
        size="sm"
        checked={remoteTransport}
        disabled={!allow}
        onChange={(v) => useHostStore.getState().setRemoteTransport(v)}
        label="Listeners can play, stop and move the playhead"
      />
      {allow && transport.kind === "wasm" && !uiSender && (
        <p className="eth-collab__hint" role="note">
          This browser can&apos;t stream your audio (it lacks WebRTC encoded transforms): others can&apos;t listen to your computer.
        </p>
      )}
      {rows.length > 0 && <p className="eth-collab__hint">Listening to you</p>}
      {rows.length > 0 && (
        <ul className="eth-collab-host__listeners" aria-label="Listening to you" data-testid="collab-listeners">
          {rows.map((r) => (
            <li key={r.key} className="eth-collab-host__listener" data-state={r.state}>
              <span>{r.name}</span>
              <Badge tone={r.state === "connected" ? "ok" : "default"}>{r.state === "connected" ? "listening" : "connecting…"}</Badge>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** "N listening" in the top bar while someone listens to this site. */
export function HostingBadge() {
  const rows = useListenerRows();
  if (rows.length === 0) return null;
  return (
    <span className="eth-collab-host__badge" title={`Listening to you: ${rows.map((r) => r.name).join(", ")}`} data-testid="collab-listening-badge">
      <Badge tone="accent">{rows.length} listening</Badge>
    </span>
  );
}
