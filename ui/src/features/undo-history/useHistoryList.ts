import { useCallback, useEffect, useState } from "react";
import type { HistoryList } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";

/**
 * The engine's undo history: listed on mount (which subscribes this client to
 * `HistoryEvent::Changed`), then kept up to date from the events and from `JumpTo` replies.
 * `null` until the first reply; `error` when the engine can't list it.
 */
export function useHistoryList(transport: EngineTransport | null) {
  const [list, setList] = useState<HistoryList | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    if (!transport) return;
    transport.send(cmd("History", { type: "List" })).then(
      (r) => {
        if (r.type === "History") {
          setList(r.history);
          setError(null);
        }
      },
      (e: unknown) => setError(e instanceof Error ? e.message : String(e)),
    );
  }, [transport]);

  useEffect(() => {
    if (!transport) return;
    refresh();
    return transport.onEvent((e) => {
      if (e.type === "History" && e.event.type === "Changed") setList(e.event.history);
      // A new project starts a new history (the engine pushes it too; listing again is cheap
      // and covers a client that connected before the event).
      else if (e.type === "ProjectLoaded") refresh();
    });
  }, [transport, refresh]);

  return { list, setList, error, setError };
}
