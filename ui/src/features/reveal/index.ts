/**
 * "Show in finder / explorer / file manager" (`Media::Reveal`): the engine resolves what to show
 * (the projects folder, a project, a sample, a clip's file) to its own path and opens the OS file
 * manager there. Only the desktop app's own engine can (its UI is on the engine machine); web and
 * remote engines reply `Unsupported`, so the actions are hidden there.
 */
import type { MediaSource, RevealTarget } from "@/generated";
import { notify } from "@/features/notifications";
import { errorMessage } from "@/features/transport-bar/engine";
import { t } from "@/i18n";
import { cmd, type EngineTransport } from "@/transport";

/** Whether `transport`'s engine can show files in this machine's file manager. */
export function canReveal(transport: EngineTransport | null | undefined): transport is EngineTransport {
  return transport?.kind === "tauri";
}

/** The OS this runs on, as far as the file manager's name goes. */
export type RevealPlatform = "mac" | "windows" | "linux";

export function revealPlatform(platform: string = typeof navigator !== "undefined" ? navigator.platform : ""): RevealPlatform {
  if (/Mac|iPhone|iPad/.test(platform)) return "mac";
  if (/Win/.test(platform)) return "windows";
  return "linux";
}

/** "show in finder" / "show in explorer" / "show in file manager". */
export function revealLabel(platform: RevealPlatform = revealPlatform()): string {
  return platform === "mac" ? t("reveal.show_in.mac") : platform === "windows" ? t("reveal.show_in.windows") : t("reveal.show_in.linux");
}

/** Show `target`; a failure (the file is gone, ...) shows as an error toast. */
export async function reveal(transport: EngineTransport, target: RevealTarget): Promise<void> {
  try {
    await transport.send(cmd("Media", { type: "Reveal", target }));
  } catch (e) {
    notify("Error", t("reveal.failed", { error: errorMessage(e) }));
  }
}

/** Show a media file (a library file, a clip's media). */
export function revealMedia(transport: EngineTransport, source: MediaSource): Promise<void> {
  return reveal(transport, { type: "Media", source });
}
