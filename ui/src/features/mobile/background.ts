/**
 * Phones in the background (#522): when the page is hidden (another app, the lock screen, a
 * call), playback stops and a recording take is finished. iOS suspends a hidden tab's
 * AudioContext anyway (the transport would keep "playing" in silence), a call should never
 * keep a take recording, and it saves the battery. Coming back does not resume.
 */
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";

type Target = Pick<Document, "visibilityState" | "addEventListener" | "removeEventListener">;

export function bindBackgroundStop(
  transport: EngineTransport,
  doc: Target = document,
  win: Pick<Window, "addEventListener" | "removeEventListener"> = window,
): () => void {
  const stop = () => {
    const state = useProjectStore.getState().transport;
    if (state?.recording) void transport.send(cmd("Recording", { type: "SetRecording", enabled: false })).catch(() => undefined);
    if (state?.playing) void transport.send(cmd("Transport", { type: "Stop" })).catch(() => undefined);
  };
  const onVisibility = () => {
    if (doc.visibilityState === "hidden") stop();
  };
  doc.addEventListener("visibilitychange", onVisibility);
  win.addEventListener("pagehide", stop);
  return () => {
    doc.removeEventListener("visibilitychange", onVisibility);
    win.removeEventListener("pagehide", stop);
  };
}
