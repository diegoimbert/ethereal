// Smooth display of a peer's pointer (docs/COLLAB.md §8.3): pointers arrive at ≤ 30 Hz, the
// overlay draws at display rate one sample interval in the past, interpolating linearly
// between the two samples around that time, and snapping when the track changes.
import type { ArrangerPointer } from "@/generated";
import { POINTER_MAX_HZ } from "./rate";

/** How far behind real time the pointer is drawn (a bit more than one sample interval). */
export const INTERP_DELAY_MS = Math.ceil(1000 / POINTER_MAX_HZ) + 7;
/** A sample arriving after this long a pause glides in over one interval instead. */
const GAP_MS = 250;
const KEEP = 6;

interface Sample {
  p: ArrangerPointer;
  t: number;
}

/** Recent samples of one peer's pointer. */
export class PointerTrail {
  private samples: Sample[] = [];

  push(p: ArrangerPointer, t: number): void {
    const last = this.samples[this.samples.length - 1];
    if (last && t - last.t > GAP_MS) {
      // After a pause, keep only the resting position, as if it was sampled just before.
      this.samples = [{ p: last.p, t: t - INTERP_DELAY_MS }];
    }
    this.samples.push({ p, t });
    if (this.samples.length > KEEP) this.samples.splice(0, this.samples.length - KEEP);
  }

  /** The newest sample. */
  latest(): ArrangerPointer | null {
    return this.samples[this.samples.length - 1]?.p ?? null;
  }

  /** The pointer to draw at `now`. */
  at(now: number): ArrangerPointer | null {
    const s = this.samples;
    if (s.length === 0) return null;
    const t = now - INTERP_DELAY_MS;
    if (t <= s[0]!.t) return s[0]!.p;
    for (let i = 1; i < s.length; i++) {
      const b = s[i]!;
      if (t >= b.t) continue;
      const a = s[i - 1]!;
      if (a.p.track !== b.p.track) return a.p;
      const k = (t - a.t) / (b.t - a.t);
      return { beats: a.p.beats + (b.p.beats - a.p.beats) * k, track: b.p.track, y: a.p.y + (b.p.y - a.p.y) * k };
    }
    return s[s.length - 1]!.p;
  }

  /** Whether `at` still moves after `now` (the overlay can stop animating otherwise). */
  settled(now: number): boolean {
    const last = this.samples[this.samples.length - 1];
    return !last || now - INTERP_DELAY_MS >= last.t;
  }
}
