// Listen-on-peer actions shared by the UI pieces: availability and the chip menu entries.
import type { CollabCommand, Presence } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { cmd, type EngineTransport } from "@/transport";
import { listenTo, stopListening } from "./agent";
import { webrtcUnsupportedReason } from "./receiver";
import { activeHost, useListenStore } from "./store";

export const send = (transport: EngineTransport) => (c: CollabCommand) => transport.send(cmd("Collab", c));

export const peerName = (p: Pick<Presence, "name">) => p.name || "Anonymous";

/** Why "Listen on <peer>" is unavailable (`null` = available). */
export function listenBlocker(peer: Presence): string | null {
  const unsupported = webrtcUnsupportedReason();
  if (unsupported) return unsupported;
  if (!peer.state.can_host) return `${peerName(peer)}'s computer cannot host a stream`;
  return null;
}

/** The chip context menu entries for `peer`. */
export function listenMenuItems(transport: EngineTransport, peer: Presence): ContextMenuEntry[] {
  const host = activeHost(useListenStore.getState().listening);
  if (host === peer.site)
    return [
      {
        label: "Stop listening",
        onSelect: () => void stopListening(send(transport)),
      },
    ];
  const blocker = listenBlocker(peer);
  return [
    {
      label: blocker ? `Listen on ${peerName(peer)}'s computer (${blocker})` : `Listen on ${peerName(peer)}'s computer`,
      disabled: blocker !== null,
      onSelect: () => void listenTo(send(transport), peer.site, peer.name),
    },
  ];
}
