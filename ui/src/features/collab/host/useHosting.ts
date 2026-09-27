// Hosting wiring ("listen on <peer>", host side; docs/COLLAB.md §9.1-§9.3): sends
// `SetHosting` when a session starts and when the preferences change, mirrors
// `ListenStatus` / `IceServers` into the host store, and runs the web sender while this UI
// streams the engine.
import { useEffect, useRef } from "react";
import { cmd, type EngineTransport } from "@/transport";
import { canSendFromUi, liveTimeline, streamOutputOf, type StreamOutput } from "./live";
import { createRtpObserver, type RtpObserver } from "./observer";
import { WebSender, type TimelineSource } from "./sender";
import { useHostStore } from "./store";

/** Browser/engine hooks of the web sender (tests replace them). */
export const hostEnv = {
  canSend: (transport: EngineTransport): boolean => canSendFromUi(transport),
  streamOutput: (transport: EngineTransport): StreamOutput | null => streamOutputOf(transport),
  createPeer: (config: RTCConfiguration): RTCPeerConnection => new RTCPeerConnection(config),
  createObserver: (): RtpObserver | null => createRtpObserver(),
  timeline: (out: StreamOutput): TimelineSource => liveTimeline(out),
};

/**
 * Hosting while in a session (`session` = its name, `null` outside one). Mount once, next
 * to the collab UI.
 */
export function useHosting(transport: EngineTransport, session: string | null): void {
  const allow = useHostStore((s) => s.allow);
  const remoteTransport = useHostStore((s) => s.remoteTransport);
  const uiSender = useHostStore((s) => s.uiSender);
  const sender = useRef<WebSender | null>(null);

  useEffect(
    () =>
      transport.onEvent((event) => {
        useHostStore.getState().onEvent(event);
        if (event.type === "Collab" && event.event.type === "Signal") {
          const { from, stream, signal } = event.event;
          sender.current?.onSignal(from, stream, signal);
        }
      }),
    [transport],
  );

  // The controller forgets the policy between sessions: declare it on every join.
  useEffect(() => {
    if (session === null) {
      useHostStore.getState().clearSession();
      return;
    }
    const ui = hostEnv.canSend(transport);
    useHostStore.getState().setUiSender(ui);
    void transport
      .send(cmd("Collab", { type: "SetHosting", allow, ui_sender: ui, remote_transport: remoteTransport }))
      .catch(() => undefined);
  }, [transport, session, allow, remoteTransport]);

  useEffect(() => {
    if (session === null || !uiSender || !allow) return;
    const out = hostEnv.streamOutput(transport);
    const observer = hostEnv.createObserver();
    if (!out || !observer) return;
    const s = new WebSender({
      send: (c) => transport.send(cmd("Collab", c)),
      createPeer: hostEnv.createPeer,
      stream: out.stream,
      observer,
      timeline: hostEnv.timeline(out),
      onChange: (links) => useHostStore.getState().setLinks(links),
    });
    sender.current = s;
    const now = useHostStore.getState();
    s.setIceServers(now.iceServers);
    s.setListeners(now.listeners);
    const off = useHostStore.subscribe((st, prev) => {
      if (st.iceServers !== prev.iceServers) s.setIceServers(st.iceServers);
      if (st.listeners !== prev.listeners) s.setListeners(st.listeners);
    });
    return () => {
      off();
      s.dispose();
      if (sender.current === s) sender.current = null;
      useHostStore.getState().setLinks(new Map());
    };
  }, [transport, session, uiSender, allow]);
}
