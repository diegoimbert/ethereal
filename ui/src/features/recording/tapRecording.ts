// Recording from a track input tap (`tap-recording`, resampling): an armed audio track whose
// input is another track records that track's signal. Only native hosts capture audio; the
// browser build cannot, so arming such a track is disabled there with this reason.
import type { TrackInput } from "@/generated";
import type { EngineTransport } from "@/transport";

/** Shown on the disabled arm button of a tapped track in the browser build. */
export const TAP_ARM_UNSUPPORTED =
  "Recording from another track needs the desktop app: the browser can't capture audio";

/**
 * Why arming a track is not possible on this host, or `null`. Only tapped tracks
 * (`TrackInput::Track`) in the browser (wasm engine) are blocked; disarming always works.
 */
export function tapArmBlocked(host: EngineTransport["kind"] | undefined, input: TrackInput, armed: boolean): string | null {
  return host === "wasm" && input.type === "Track" && !armed ? TAP_ARM_UNSUPPORTED : null;
}
