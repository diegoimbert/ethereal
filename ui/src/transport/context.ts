import { createContext, useContext, useEffect, useRef } from "react";
import type { Event } from "@/generated";
import type { EngineTransport } from "./EngineTransport";

export type ConnectionStatus =
  | { status: "connecting" }
  | { status: "connected" }
  | { status: "error"; error: unknown };

export interface TransportContextValue {
  transport: EngineTransport;
  connection: ConnectionStatus;
}

export const TransportContext = createContext<TransportContextValue | null>(null);

function useTransportContext(): TransportContextValue {
  const ctx = useContext(TransportContext);
  if (!ctx) throw new Error("useTransport() must be used inside <TransportProvider>");
  return ctx;
}

/** The engine transport. Send commands with `useTransport().send(cmd(...))`. */
export function useTransport(): EngineTransport {
  return useTransportContext().transport;
}

/** Connection state of the transport (connecting → connected | error). */
export function useConnectionStatus(): ConnectionStatus {
  return useTransportContext().connection;
}

/**
 * Subscribe to raw engine events (e.g. `Plugin`, `Media`, `Notification`) for the lifetime
 * of the component. Document/transport events are already mirrored into the
 * project store by `TransportProvider`; prefer reading the store for those.
 */
export function useTransportEvent(listener: (event: Event) => void): void {
  const transport = useTransport();
  const ref = useRef(listener);
  useEffect(() => {
    ref.current = listener;
  });
  useEffect(() => transport.onEvent((e) => ref.current(e)), [transport]);
}
