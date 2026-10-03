import "./capture.css";
import { useContext, useState } from "react";
import { KeyboardMusic } from "lucide-react";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { isCommandFailed, TransportContext } from "@/transport";
import { captureMidi } from "./actions";
import { captureTitle, useCaptureAvailable } from "./availability";

/**
 * Transport-bar "Capture" button: always listening, it turns what was just played into a
 * clip on the selected (or armed) MIDI track. Lit while there is something to capture.
 */
export function CaptureButton({ className }: { className?: string }) {
  const transport = useContext(TransportContext)?.transport ?? null;
  const hasProject = useProjectStore((s) => s.project !== null);
  const available = useCaptureAvailable();
  const [error, setError] = useState<string | null>(null);
  const run = () => {
    if (!transport) return;
    setError(null);
    captureMidi(transport).catch((e: unknown) => setError(isCommandFailed(e) ? e.error.message : String(e)));
  };
  return (
    <Button
      tone="ghost"
      aria-label="Capture MIDI"
      title={error ?? captureTitle(available)}
      className={className ? `${className} eth-capture` : "eth-capture"}
      data-available={available || undefined}
      data-testid="capture-midi"
      disabled={!transport || !hasProject || !available}
      onClick={run}
    >
      <KeyboardMusic aria-hidden />
    </Button>
  );
}
