/**
 * Per-client-IP budgets (docs/SHARING.md §3.4): failed `JoinHello`s per minute and room
 * claims per hour. A sliding window of timestamps per (kind, ip), in memory.
 *
 * On Cloudflare one `IpBudget` Durable Object per IP holds these (so the limit is global, not
 * per colo); the Node adapter and the tests use the class directly.
 */

import { LIMITS } from "./protocol.ts";

export type BudgetKind = "claim" | "badDoor";

/** `peek`: is there budget left? `spend`: use one unit, then answer the same question. */
export type BudgetOp = "peek" | "spend";

const WINDOW: Record<BudgetKind, { max: number; ms: number }> = {
  claim: { max: LIMITS.claimsPerHour, ms: 3_600_000 },
  badDoor: { max: LIMITS.badDoorsPerMinute, ms: 60_000 },
};

export class IpBudgets {
  private readonly hits: Map<string, number[]>;

  /** `saved`: a previous `toJSON()` (the `IpBudget` Durable Object stores it). */
  constructor(saved?: Record<string, number[]>) {
    this.hits = new Map(Object.entries(saved ?? {}));
  }

  toJSON(): Record<string, number[]> {
    return Object.fromEntries(this.hits);
  }

  /**
   * `true` while `ip` is under the limit for `kind`. `spend` records one use first, so the
   * call that reaches the limit already answers `false`.
   */
  check(kind: BudgetKind, ip: string, op: BudgetOp, now: number): boolean {
    const { max, ms } = WINDOW[kind];
    const key = `${kind}:${ip}`;
    const recent = (this.hits.get(key) ?? []).filter((t) => t > now - ms);
    if (op === "spend") recent.push(now);
    if (recent.length) this.hits.set(key, recent);
    else this.hits.delete(key);
    return op === "spend" ? recent.length <= max : recent.length < max;
  }

  /** Forget windows that expired (bounded memory for a long-lived process). */
  sweep(now: number): void {
    for (const [key, times] of this.hits) {
      const kind = key.slice(0, key.indexOf(":")) as BudgetKind;
      if (!times.some((t) => t > now - WINDOW[kind].ms)) this.hits.delete(key);
    }
  }
}
