/**
 * Mock of `Groove::{Humanize, SetSwing}` and the quantize swing law (used by the core
 * reducer's `Note::Quantize`). Owned by `groove`.
 */

import type { GrooveCommand } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";
import { mulberry32 } from "../random";
import { clamp } from "./shared";

export function grooveCommand(ctx: ReducerContext, c: GrooveCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "Humanize": {
      if (!tx.get("Clip", c.clip)) fail("NotFound", `clip ${c.clip}`);
      const ids = c.notes ? new Set(c.notes) : null;
      const rand = mulberry32(c.seed);
      const notes = tx
        .all("Note")
        .filter((n) => n.clip === c.clip && (!ids || ids.has(n.id)))
        .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
      for (const n of notes) {
        const dt = (rand() * 2 - 1) * Math.max(0, c.timing);
        const dv = (rand() * 2 - 1) * Math.max(0, c.velocity);
        tx.upsert("Note", { ...n, start: Math.max(0, n.start + dt), velocity: clamp(n.velocity + dv, 0, 1) });
      }
      break;
    }
    case "SetSwing":
      if (!(c.grid > 0)) fail("InvalidArgument", "swing grid must be > 0");
      tx.setSettings({ ...tx.project.settings, swing: clamp(c.amount, 0, 1), swing_grid: c.grid });
      break;
  }
}

/** Swing delay for a quantize target `t` on `grid`: odd grid positions move by `swing·grid/3`. */
export function swingOffset(t: number, grid: number, swing: number): number {
  const index = Math.round(t / grid);
  return Math.abs(index % 2) === 1 ? (clamp(swing, 0, 1) * grid) / 3 : 0;
}
