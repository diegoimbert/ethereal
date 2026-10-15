import { rectOf } from "./geometry";
import type { TourContext, TourEnv, TourPlatform, TourStep } from "./types";

/** The elements a step highlights (only the ones laid out). */
export function findTargets(step: TourStep | null, ctx: TourContext | null, doc: Document = document): Element[] {
  if (!step?.targets || !ctx) return [];
  const els = typeof step.targets === "function" ? step.targets(ctx) : step.targets.flatMap((name) => [...doc.querySelectorAll(`[data-tour="${name}"]`)]);
  return els.filter((el) => rectOf(el) !== null);
}

/**
 * `data-tour` targets that only exist on some hosts: Settings > Audio shows the device fields
 * only where the engine lists devices (the desktop app), and its "unavailable" note elsewhere.
 * A step pointing only at such targets needs a `when` guard (or a fallback) on the other
 * hosts; tours.static.test.ts checks it. Targets not listed here exist everywhere.
 */
export const HOST_TARGETS: Readonly<Record<string, (env: TourEnv) => boolean>> = {
  "audio-driver": (env) => env.devicesListable,
  "audio-output": (env) => env.devicesListable,
  "audio-input": (env) => env.devicesListable,
  "audio-sample-rate": (env) => env.devicesListable,
  "audio-buffer": (env) => env.devicesListable,
  "audio-status": (env) => env.devicesListable,
  "audio-input-check": (env) => env.devicesListable,
  "audio-unavailable": (env) => !env.devicesListable,
};

/** The usual host of each platform (what `detectEnv` measures there). */
export const PLATFORM_ENVS: Readonly<Record<TourPlatform, TourEnv>> = {
  desktop: { host: "tauri", canRecord: true, devicesListable: true },
  web: { host: "wasm", canRecord: false, devicesListable: false },
  mobile: { host: "wasm", canRecord: false, devicesListable: false },
};

/** The platform a host's welcome screen is on (the mobile shell mounts no tour overlay). */
export const platformOf = (host: TourEnv["host"]): TourPlatform => (host === "tauri" ? "desktop" : "web");
