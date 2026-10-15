/**
 * Geometry of an FM algorithm diagram (`Widget::FmAlgorithm`, fm-synth): operators as
 * boxes stacked by depth (carriers on the bottom row, each modulator one row above the
 * highest operator it modulates), modulation edges from a modulator down to its targets
 * and carrier edges down to the output bar. Pure, in a unit box (0..1 both axes) that the
 * widget scales; no DOM.
 */

import type { FmRouting } from "@/generated";

export interface FmNode {
  /** Operator index (0 = "A"). */
  op: number;
  name: string;
  /** Box centre, 0..1 (y grows downward). */
  x: number;
  y: number;
  carrier: boolean;
}

export interface FmEdge {
  from: number;
  to: number;
}

export interface FmDiagram {
  nodes: FmNode[];
  /** Modulator → modulated operator. */
  edges: FmEdge[];
  /** Rows of boxes (the bottom one holds the carriers). */
  rows: number;
  /** y of the output bar (below the carriers). */
  outputY: number;
}

const OP_NAMES = "ABCDEFGH";

const has = (mask: number, bit: number) => (mask & (1 << bit)) !== 0;

/** Lay out `routing`. Modulation cycles (never in factory tables) are cut at the second visit. */
export function fmDiagram(routing: FmRouting): FmDiagram {
  const n = routing.modulators.length;
  // Depth: 0 for carriers, else 1 + the deepest operator it modulates.
  const depth = new Array<number>(n).fill(-1);
  const visiting = new Set<number>();
  const depthOf = (op: number): number => {
    if (depth[op]! >= 0) return depth[op]!;
    if (visiting.has(op)) return 0;
    visiting.add(op);
    let d = has(routing.carriers, op) ? 0 : -1;
    for (let t = 0; t < n; t++) {
      if (t !== op && has(routing.modulators[t] ?? 0, op)) d = Math.max(d, depthOf(t) + 1);
    }
    visiting.delete(op);
    depth[op] = Math.max(0, d);
    return depth[op]!;
  };
  for (let op = 0; op < n; op++) depthOf(op);
  const rows = Math.max(1, ...depth.map((d) => d + 1));
  // Each row's operators in index order, spread across the width.
  const byRow = new Map<number, number[]>();
  for (let op = 0; op < n; op++) byRow.set(depth[op]!, [...(byRow.get(depth[op]!) ?? []), op]);
  // Rows from the top (deepest) to the carriers, leaving the bottom band for the output.
  const rowY = (d: number) => (rows - 1 - d + 0.5) / (rows + 0.5);
  const nodes: FmNode[] = [];
  for (const [d, ops] of byRow) {
    ops.forEach((op, i) =>
      nodes.push({
        op,
        name: OP_NAMES[op] ?? String(op + 1),
        x: (i + 0.5) / ops.length,
        y: rowY(d),
        carrier: has(routing.carriers, op),
      }),
    );
  }
  nodes.sort((a, b) => a.op - b.op);
  const edges: FmEdge[] = [];
  routing.modulators.forEach((mask, to) => {
    for (let from = 0; from < n; from++) if (from !== to && has(mask, from)) edges.push({ from, to });
  });
  return { nodes, edges, rows, outputY: (rows + 0.15) / (rows + 0.5) };
}
