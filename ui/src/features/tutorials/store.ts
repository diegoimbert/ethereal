// The tour engine: which tour runs, its current step, and how steps move on. The overlay
// (`TourRoot`) renders it and calls `tickTour` every frame with what it found on screen.
import { create } from "zustand";
import { markTourDone } from "./progress";
import type { TourAction, TourContext, TourDef, TourStep } from "./types";

/** How long a step's highlighted parts may be missing before its fallback shows (ms). */
export const MISSING_GRACE_MS = 1200;

/** Why a tour ended: finished (the last step's "finish", or one of its actions), or left. */
export type TourEnd = "finished" | "exited" | "action";

export interface TourState {
  tour: TourDef | null;
  ctx: TourContext | null;
  /** Index of the current step in `tour.steps`. */
  index: number;
  /** How the current step was entered: going back to a waiting step doesn't skip it. */
  entered: "forward" | "back";
  /** The waiting step's condition, as last checked. */
  satisfied: boolean;
  /** The step's highlighted parts can't be found: its fallback shows. */
  missing: boolean;
  missingSince: number | null;
  /** "Exit the tutorial?" is asked in the bubble. */
  confirmExit: boolean;
  /** Bumped when a click is blocked (the bubble draws attention to itself). */
  nudge: number;
  /** Mounted overlays (`TourRoot`): a tour only starts where one can show it. */
  hosts: number;
  /** Called once when the tour ends. */
  onEnd: ((end: TourEnd) => void) | null;
}

const IDLE: Omit<TourState, "hosts"> = {
  tour: null,
  ctx: null,
  index: 0,
  entered: "forward",
  satisfied: false,
  missing: false,
  missingSince: null,
  confirmExit: false,
  nudge: 0,
  onEnd: null,
};

export const useTour = create<TourState>()(() => ({ ...IDLE, hosts: 0 }));

const shows = (step: TourStep, ctx: TourContext) => step.when?.(ctx) ?? true;

/** The steps shown with this context (the ones whose `when` holds). */
export function visibleSteps(tour: TourDef, ctx: TourContext): TourStep[] {
  return tour.steps.filter((s) => shows(s, ctx));
}

function findShown(tour: TourDef, ctx: TourContext, from: number, dir: 1 | -1): number {
  for (let i = from; i >= 0 && i < tour.steps.length; i += dir) if (shows(tour.steps[i]!, ctx)) return i;
  return -1;
}

export function currentStep(s: Pick<TourState, "tour" | "index"> = useTour.getState()): TourStep | null {
  return s.tour?.steps[s.index] ?? null;
}

/** Whether "next" may be pressed: not while a step waits for the app (unless its parts are missing). */
export function canAdvance(s: TourState = useTour.getState()): boolean {
  const step = currentStep(s);
  if (!step) return false;
  return !step.until || s.satisfied || s.missing;
}

/** Enter step `index`. Going forward, a waiting step that is already satisfied is skipped. */
function enter(index: number, entered: "forward" | "back"): void {
  const { tour, ctx } = useTour.getState();
  if (!tour || !ctx) return;
  let i = index;
  for (;;) {
    const step = tour.steps[i]!;
    const satisfied = step.until?.(ctx) ?? false;
    const next = findShown(tour, ctx, i + 1, 1);
    if (entered === "forward" && step.until && satisfied && next >= 0) {
      i = next;
      continue;
    }
    useTour.setState({ index: i, entered, satisfied, missing: false, missingSince: null, confirmExit: false });
    try {
      step.onEnter?.(ctx);
    } catch (e) {
      console.warn("tour step setup failed", e);
    }
    return;
  }
}

export interface StartOptions {
  onEnd?(end: TourEnd): void;
}

export function startTour(tour: TourDef, ctx: TourContext, opts: StartOptions = {}): void {
  const first = findShown(tour, ctx, 0, 1);
  if (first < 0) return;
  useTour.setState({ ...IDLE, tour, ctx, onEnd: opts.onEnd ?? null });
  enter(first, "forward");
}

function end(how: TourEnd): void {
  const { onEnd, tour, ctx } = useTour.getState();
  try {
    if (tour && ctx) tour.cleanup?.(ctx);
  } catch (e) {
    console.warn("tour cleanup failed", e);
  }
  useTour.setState({ ...IDLE });
  onEnd?.(how);
}

/** "next": the next shown step, or the end of the tour (done). */
export function nextStep(): void {
  const { tour, ctx, index } = useTour.getState();
  if (!tour || !ctx) return;
  const next = findShown(tour, ctx, index + 1, 1);
  if (next < 0) finishTour();
  else enter(next, "forward");
}

export function backStep(): void {
  const { tour, ctx, index } = useTour.getState();
  if (!tour || !ctx) return;
  const prev = findShown(tour, ctx, index - 1, -1);
  if (prev >= 0) enter(prev, "back");
}

/** Done: remembered as completed. With an action, it runs (and the welcome screen stays away). */
export function finishTour(action?: TourAction): void {
  const { tour, ctx } = useTour.getState();
  if (!tour || !ctx) return;
  markTourDone(tour);
  end(action ? "action" : "finished");
  if (action) void Promise.resolve(action.run(ctx)).catch((e: unknown) => console.warn("tour action failed", e));
}

/** Leave without finishing (not marked done). */
export function exitTour(): void {
  if (useTour.getState().tour) end("exited");
}

export function askExit(confirm: boolean): void {
  if (useTour.getState().tour) useTour.setState({ confirmExit: confirm });
}

export function nudgeTour(): void {
  useTour.setState((s) => ({ nudge: s.nudge + 1 }));
}

/**
 * Once per frame: `found` is how many of the step's highlighted parts are on screen (`null`
 * for a step without any). Moves on when a waiting step's condition becomes true; shows the
 * fallback once the parts have been missing for `MISSING_GRACE_MS`.
 */
export function tickTour(now: number, found: number | null): void {
  const s = useTour.getState();
  const step = currentStep(s);
  if (!step || !s.ctx) return;
  const patch: Partial<TourState> = {};
  if (found === 0) {
    const since = s.missingSince ?? now;
    if (s.missingSince === null) patch.missingSince = now;
    if (!s.missing && now - since >= MISSING_GRACE_MS) patch.missing = true;
  } else if (s.missing || s.missingSince !== null) {
    patch.missing = false;
    patch.missingSince = null;
  }
  if (step.until) {
    const sat = step.until(s.ctx);
    if (sat && !s.satisfied) {
      nextStep();
      return;
    }
    if (sat !== s.satisfied) patch.satisfied = sat;
  }
  if (Object.keys(patch).length > 0) useTour.setState(patch);
}

/** Tests: back to idle. */
export function resetTour(): void {
  useTour.setState({ ...IDLE });
}
