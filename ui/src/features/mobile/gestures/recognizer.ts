/**
 * The touch gesture recognizer (#522): a pure state machine fed the touches of one surface
 * (`down` / `move` / `up` / `cancel`, plus `timer` when the long-press delay is due) that
 * reports gestures. No DOM and no clock of its own: `touchSurface.ts` binds it to pointer
 * events and a timeout; the tests drive it directly.
 *
 * One finger:
 *   - released before `SLOP_PX` and `LONG_PRESS_MS`: `tap` (`count` 2 for a double-tap);
 *   - held `LONG_PRESS_MS` without moving: `longpress`, then on release `longpress-up`;
 *   - moved past `SLOP_PX`: `pan-start` (`after` "press", or "longpress" when held first; the
 *     axis locks when the first movement is mostly along one), `pan` (incremental), `pan-end`
 *     (with the release speed, for momentum).
 * Two fingers: a second finger ends any one-finger gesture with `cancel` (a drag drops its
 * preview), then `pinch-start` (its axis from how the fingers lie: side by side zooms time,
 * one above the other zooms height), `pinch` (incremental factor and midpoint motion),
 * `pinch-end` when one lifts. Fingers left after a pinch are ignored until all are up (no
 * jump into a pan). A third finger is ignored.
 */
import { AXIS_LOCK_RATIO, DOUBLE_TAP_MS, DOUBLE_TAP_PX, LONG_PRESS_MS, MIN_PINCH_SPREAD_PX, SLOP_PX } from "./constants";
import { VelocityTracker } from "./momentum";

export interface Point {
  x: number;
  y: number;
}

export type Axis = "x" | "y" | "free";

export type Gesture =
  | { type: "tap"; x: number; y: number; count: 1 | 2 }
  | { type: "longpress"; x: number; y: number }
  | { type: "longpress-up"; x: number; y: number }
  | { type: "pan-start"; x: number; y: number; startX: number; startY: number; axis: Axis; after: "press" | "longpress" }
  | { type: "pan"; x: number; y: number; dx: number; dy: number }
  | { type: "pan-end"; x: number; y: number; vx: number; vy: number }
  | { type: "pinch-start"; x: number; y: number; axis: "x" | "y" }
  | { type: "pinch"; x: number; y: number; axis: "x" | "y"; scale: number; dx: number; dy: number }
  | { type: "pinch-end" }
  | { type: "cancel" };

type State =
  | { kind: "idle" }
  | { kind: "pressed"; id: number; start: Point; t0: number; last: Point }
  | { kind: "held"; id: number; start: Point; last: Point }
  | { kind: "panning"; id: number; last: Point; axis: Axis }
  | { kind: "pinching"; ids: [number, number]; axis: "x" | "y"; spread: number; mid: Point }
  | { kind: "ignoring" };

export class GestureRecognizer {
  private state: State = { kind: "idle" };
  private readonly points = new Map<number, Point>();
  private lastTap: { at: Point; t: number } | null = null;
  private readonly velocity = new VelocityTracker();

  constructor(private readonly emit: (g: Gesture) => void) {}

  /** A long-press timer should run from now (the binding schedules `timer` after `LONG_PRESS_MS`). */
  get waitingForLongPress(): boolean {
    return this.state.kind === "pressed";
  }

  get active(): boolean {
    return this.state.kind !== "idle";
  }

  down(id: number, p: Point, t: number): void {
    this.points.set(id, p);
    const s = this.state;
    if (s.kind === "idle") {
      this.state = { kind: "pressed", id, start: p, t0: t, last: p };
      this.velocity.reset(p, t);
      return;
    }
    if (s.kind === "pressed" || s.kind === "held" || s.kind === "panning") {
      const first = this.points.get(s.id);
      // A second finger: whatever one finger was doing is cancelled, and a pinch begins.
      if (s.kind !== "pressed") this.emit({ type: "cancel" });
      if (!first) {
        this.state = { kind: "ignoring" };
        return;
      }
      const axis: "x" | "y" = Math.abs(p.x - first.x) >= Math.abs(p.y - first.y) ? "x" : "y";
      const mid = midpoint(first, p);
      this.state = { kind: "pinching", ids: [s.id, id], axis, spread: spread(first, p, axis), mid };
      this.emit({ type: "pinch-start", x: mid.x, y: mid.y, axis });
    }
    // A third finger, or one after a pinch: ignored.
  }

  move(id: number, p: Point, t: number): void {
    if (!this.points.has(id)) return;
    this.points.set(id, p);
    const s = this.state;
    switch (s.kind) {
      case "pressed":
      case "held": {
        if (id !== s.id) return;
        const dx = p.x - s.start.x;
        const dy = p.y - s.start.y;
        this.velocity.add(p, t);
        if (Math.hypot(dx, dy) < SLOP_PX) {
          s.last = p;
          return;
        }
        const axis: Axis = s.kind === "held" ? "free" : lockAxis(dx, dy);
        this.state = { kind: "panning", id, last: p, axis };
        this.emit({ type: "pan-start", x: p.x, y: p.y, startX: s.start.x, startY: s.start.y, axis, after: s.kind === "held" ? "longpress" : "press" });
        // The movement past the slop is applied at once (no dead zone in the pan).
        const step = constrain(axis, dx, dy);
        if (step.dx !== 0 || step.dy !== 0) this.emit({ type: "pan", x: p.x, y: p.y, ...step });
        return;
      }
      case "panning": {
        if (id !== s.id) return;
        this.velocity.add(p, t);
        const step = constrain(s.axis, p.x - s.last.x, p.y - s.last.y);
        s.last = p;
        if (step.dx !== 0 || step.dy !== 0) this.emit({ type: "pan", x: p.x, y: p.y, ...step });
        return;
      }
      case "pinching": {
        if (!s.ids.includes(id)) return;
        const a = this.points.get(s.ids[0]);
        const b = this.points.get(s.ids[1]);
        if (!a || !b) return;
        const next = spread(a, b, s.axis);
        const mid = midpoint(a, b);
        const scale = next / s.spread;
        const dx = mid.x - s.mid.x;
        const dy = mid.y - s.mid.y;
        s.spread = next;
        s.mid = mid;
        if (scale !== 1 || dx !== 0 || dy !== 0) this.emit({ type: "pinch", x: mid.x, y: mid.y, axis: s.axis, scale, dx, dy });
        return;
      }
      default:
        return;
    }
  }

  up(id: number, p: Point, t: number): void {
    if (!this.points.has(id)) return;
    this.points.delete(id);
    const s = this.state;
    switch (s.kind) {
      case "pressed": {
        if (id !== s.id) break;
        const double =
          this.lastTap !== null && t - this.lastTap.t <= DOUBLE_TAP_MS && Math.hypot(p.x - this.lastTap.at.x, p.y - this.lastTap.at.y) <= DOUBLE_TAP_PX;
        this.emit({ type: "tap", x: s.start.x, y: s.start.y, count: double ? 2 : 1 });
        // A third tap starts a new pair.
        this.lastTap = double ? null : { at: s.start, t };
        this.state = { kind: "idle" };
        break;
      }
      case "held":
        if (id !== s.id) break;
        this.emit({ type: "longpress-up", x: s.start.x, y: s.start.y });
        this.lastTap = null;
        this.state = { kind: "idle" };
        break;
      case "panning": {
        if (id !== s.id) break;
        this.velocity.add(p, t);
        const v = this.velocity.speed(t);
        const c = constrain(s.axis, v.x, v.y);
        this.emit({ type: "pan-end", x: p.x, y: p.y, vx: c.dx, vy: c.dy });
        this.lastTap = null;
        this.state = { kind: "idle" };
        break;
      }
      case "pinching":
        if (!s.ids.includes(id)) break;
        this.emit({ type: "pinch-end" });
        this.lastTap = null;
        this.state = { kind: "ignoring" };
        break;
      default:
        break;
    }
    if (this.points.size === 0 && this.state.kind === "ignoring") this.state = { kind: "idle" };
  }

  /** The long-press delay elapsed (the binding's timeout, `LONG_PRESS_MS` after `down`). */
  timer(t: number): void {
    const s = this.state;
    if (s.kind !== "pressed" || t - s.t0 < LONG_PRESS_MS) return;
    this.state = { kind: "held", id: s.id, start: s.start, last: s.last };
    this.emit({ type: "longpress", x: s.start.x, y: s.start.y });
  }

  /** A touch elsewhere (a button): the next tap here starts a new pair, never a double-tap. */
  forgetTap(): void {
    this.lastTap = null;
  }

  /** The system took the touches (pointercancel): end whatever was going on. */
  cancel(): void {
    const s = this.state;
    if (s.kind === "held" || s.kind === "panning") this.emit({ type: "cancel" });
    else if (s.kind === "pinching") this.emit({ type: "pinch-end" });
    this.points.clear();
    this.state = { kind: "idle" };
  }
}

function lockAxis(dx: number, dy: number): Axis {
  const ax = Math.abs(dx);
  const ay = Math.abs(dy);
  if (ax >= AXIS_LOCK_RATIO * ay) return "x";
  if (ay >= AXIS_LOCK_RATIO * ax) return "y";
  return "free";
}

function constrain(axis: Axis, dx: number, dy: number): { dx: number; dy: number } {
  return { dx: axis === "y" ? 0 : dx, dy: axis === "x" ? 0 : dy };
}

const midpoint = (a: Point, b: Point): Point => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });

function spread(a: Point, b: Point, axis: "x" | "y"): number {
  return Math.max(MIN_PINCH_SPREAD_PX, Math.abs(axis === "x" ? a.x - b.x : a.y - b.y));
}
