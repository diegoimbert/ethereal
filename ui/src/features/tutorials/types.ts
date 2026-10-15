// Guided tours (docs/TUTORIALS.md): a tour is data, a list of steps. Each step points at
// parts of the real UI (their `data-tour="…"` attributes), says something about them in a
// bubble, and moves on when the user presses "next" or when the app reaches a state (the
// settings opened, a track got armed, …).
import type { ReactNode } from "react";
import type { MessageKey } from "@/i18n";
import type { EngineTransport } from "@/transport";

/** What the host can do, measured when a tour starts (copy and steps branch on it). */
export interface TourEnv {
  /** The engine's host: "tauri" (desktop), "wasm" (browser), "remote" (an engine server), "mock". */
  host: EngineTransport["kind"] | null;
  /** The engine can record audio (`Recording::ListInputs` answers; the browser build can't). */
  canRecord: boolean;
  /** The engine lists audio devices (Settings > Audio shows the device choices; not in the browser). */
  devicesListable: boolean;
}

/** Passed to a step's predicates and actions. */
export interface TourContext {
  transport: EngineTransport | null;
  env: TourEnv;
}

/** Where the bubble goes, relative to the highlighted area ("auto": the side with the most room). */
export type TourPlacement = "auto" | "top" | "bottom" | "left" | "right" | "center";

/** A choice offered on a step besides "next" (the last steps: "start a session now", "pick a file now"). */
export interface TourAction {
  id: string;
  /** Message keys (docs/LOCALIZATION.md): shown with `t` when the bubble renders. */
  label: MessageKey;
  /**
   * Runs after the tour is marked done and closed. The welcome screen does not come back:
   * the user chose to go on in the app.
   */
  run(ctx: TourContext): void | Promise<void>;
}

export type TourText = ReactNode | ((ctx: TourContext) => ReactNode);

export interface TourStep {
  id: string;
  /**
   * The highlighted parts: `data-tour` names (every element carrying one of them), or a
   * function finding the elements (a track's header, …). None: a centered bubble.
   */
  targets?: ReadonlyArray<string> | ((ctx: TourContext) => Element[]);
  title: MessageKey;
  /** Its text: `<Trans>` elements (`tours/text.tsx`), formatted when shown. */
  body: TourText;
  /** A drawing above the text (a diagram). */
  illustration?: ReactNode;
  placement?: TourPlacement;
  /**
   * Wait for the app instead of a "next" button: the step moves on as soon as this becomes
   * true (checked on every frame; read the stores' `getState()` or the DOM).
   */
  until?(ctx: TourContext): boolean;
  /** What to do to move on (under the text of a waiting step), e.g. "click the gear". */
  hint?: MessageKey;
  /**
   * The highlighted parts take clicks (default: only on waiting steps). Elsewhere the screen
   * never does.
   */
  interactive?: boolean;
  /** Let the transport keys (space: play / stop) work during this step. */
  transportKeys?: boolean;
  /** Only shown when this is true (a feature the host lacks is skipped). */
  when?(ctx: TourContext): boolean;
  /**
   * Shown instead of `body`, in a centered bubble with "next", when the highlighted parts
   * can't be found (a missing feature on this platform): a tour never gets stuck.
   */
  fallback?: TourText;
  /** Runs when the step is entered (either way): open or close what the step needs. */
  onEnter?(ctx: TourContext): void;
  /** Extra choices (the last step's "start a session now"). */
  actions?: ReadonlyArray<TourAction>;
}

/**
 * Where a tour is offered: "desktop" (the desktop app), "web" (the browser app, wasm engine),
 * "mobile" (the mobile shell, which mounts no tour overlay yet). Steps that need a desktop-only
 * part of the UI are guarded with `when` (tours.static.test.ts checks it against
 * `HOST_TARGETS`).
 */
export type TourPlatform = "desktop" | "web" | "mobile";

export interface TourDef {
  /** Stable id (the completion record is `${id}@${version}`). */
  id: string;
  /** The platforms the tour is offered on (the welcome screen shows it as "soon" elsewhere). */
  platforms: ReadonlyArray<TourPlatform>;
  /** Bump when the tour is rewritten: its old completion no longer counts. */
  version: number;
  title: MessageKey;
  /** The project made for the tour ("jam tutorial", in the user's language), kept in the user's projects. */
  projectName: MessageKey;
  /**
   * "starter": the default new project (a synth track); "empty": no tracks; `{ template }`: a
   * project made from that project template (the example beats: `factory/example-…`).
   */
  project: "starter" | "empty" | { template: string };
  steps: ReadonlyArray<TourStep>;
  /** Runs when the tour ends, however it ends: close what its steps opened. */
  cleanup?(ctx: TourContext): void;
}

export const textOf = (t: TourText, ctx: TourContext): ReactNode => (typeof t === "function" ? t(ctx) : t);
