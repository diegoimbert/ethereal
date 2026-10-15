import { useEffect } from "react";
import type { EngineTransport } from "@/transport";
import { onNotificationEvent, useNotices } from "./store";

/** Turn the engine's `Event::Notification`s into notices while mounted. */
export function useEngineNotifications(transport: EngineTransport | null): void {
  useEffect(() => transport?.onEvent(onNotificationEvent), [transport]);
}

/** Whether there is anything to show (the shared stack renders only then). */
export function useHasNotices(): boolean {
  return useNotices((s) => s.notices.length > 0);
}
