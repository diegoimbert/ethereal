// Small engine-access helpers shared by the ui-shell features (transport bar, project
// manager, browser). They tolerate a missing `<TransportProvider>` (e.g. the bare App shell
// in tests): controls then render disabled instead of throwing.
import { useCallback, useContext, useEffect, useRef, useState } from "react";
import type { Command, Event, ReplyValue } from "@/generated";
import { isCommandFailed, TransportContext, type ConnectionStatus, type EngineTransport, type SendOptions } from "@/transport";

/** The engine transport, or `null` outside a `<TransportProvider>`. */
export function useOptionalTransport(): EngineTransport | null {
  return useContext(TransportContext)?.transport ?? null;
}

/** Connection status, or `null` outside a `<TransportProvider>`. */
export function useOptionalConnection(): ConnectionStatus | null {
  return useContext(TransportContext)?.connection ?? null;
}

/** Human-readable message of a failed command. */
export function errorMessage(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message || e.error.code;
  if (e instanceof Error) return e.message;
  return String(e);
}

export interface EngineCommands {
  transport: EngineTransport | null;
  /** Send a command. Resolves with the reply, or `undefined` if it failed (see `error`). */
  send(command: Command, opts?: SendOptions): Promise<ReplyValue | undefined>;
  /** Message of the last failed command (cleared by the next successful one). */
  error: string | null;
  clearError(): void;
}

/** `send` wrapper that turns failures into a displayable `error` instead of throwing. */
export function useEngineCommands(): EngineCommands {
  const transport = useOptionalTransport();
  const [error, setError] = useState<string | null>(null);
  const send = useCallback(
    async (command: Command, opts?: SendOptions): Promise<ReplyValue | undefined> => {
      if (!transport) return undefined;
      try {
        const reply = await transport.send(command, opts);
        setError(null);
        return reply;
      } catch (e) {
        setError(errorMessage(e));
        return undefined;
      }
    },
    [transport],
  );
  const clearError = useCallback(() => setError(null), []);
  return { transport, send, error, clearError };
}

/** Subscribe to raw engine events while mounted (no-op without a transport). */
export function useEngineEvent(listener: (event: Event) => void): void {
  const transport = useOptionalTransport();
  const ref = useRef(listener);
  useEffect(() => {
    ref.current = listener;
  });
  useEffect(() => transport?.onEvent((e) => ref.current(e)), [transport]);
}

/** `true` if a key event comes from a text field (shortcuts must not fire there). */
export function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}
