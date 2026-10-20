/**
 * Whether the host can record: `Recording::ListInputs` replies `Unsupported` where
 * there are no audio or MIDI inputs (the browser build). The desktop record button and the
 * mobile transport's both disable themselves with `unsupportedTooltip()` then.
 */
import { useEffect, useState } from "react";
import { cmd, isCommandFailed, type EngineTransport } from "@/transport";
import { t } from "@/i18n";

/** Shown on the disabled record button when the host has no inputs (browser build). */
export const unsupportedTooltip = (): string => t("recording.unsupported");

/**
 * `true` once `transport` said it has no inputs. Probed when connected with a project open
 * (`ready`); any other failure counts as supported (the record button stays usable).
 */
export function useRecordingUnsupported(transport: EngineTransport | null, ready: boolean): boolean {
  const [unsupported, setUnsupported] = useState(false);
  useEffect(() => {
    if (!transport || !ready) return;
    let active = true;
    transport.send(cmd("Recording", { type: "ListInputs" })).then(
      () => active && setUnsupported(false),
      (e: unknown) => active && setUnsupported(isCommandFailed(e) && e.error.code === "Unsupported"),
    );
    return () => {
      active = false;
    };
  }, [transport, ready]);
  return unsupported;
}
