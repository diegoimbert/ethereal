import { useContext, useEffect, useState } from "react";
import { useProjectStore } from "@/state";
import { cmd, TransportContext } from "@/transport";

/**
 * Whether the engine's capture buffer holds something to capture: `Capture::Status` on
 * connect / project change, then `Event::Capture { Changed }`. `false` without an engine.
 */
export function useCaptureAvailable(): boolean {
  const transport = useContext(TransportContext)?.transport ?? null;
  const projectId = useProjectStore((s) => s.project?.id ?? null);
  const [available, setAvailable] = useState(false);
  useEffect(() => {
    if (!transport) return;
    let active = true;
    transport.send(cmd("Capture", { type: "Status" })).then(
      (reply) => {
        if (active && reply.type === "CaptureStatus") setAvailable(reply.status.available);
      },
      () => undefined, // hosts without capture: stays unavailable
    );
    const off = transport.onEvent((e) => {
      if (e.type === "Capture" && e.event.type === "Changed") setAvailable(e.event.status.available);
    });
    return () => {
      active = false;
      off();
    };
  }, [transport, projectId]);
  return available;
}

/** Tooltip for the Capture button. */
export function captureTitle(available: boolean): string {
  return available ? "Capture MIDI: turn what you just played into a clip" : "Capture MIDI (play something first)";
}
