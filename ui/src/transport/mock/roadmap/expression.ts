/**
 * Mock of `Expression::*` (v0.3, contracts-4). Owned by `midi-expression`: clip expression
 * lanes (CC / pitch bend / channel pressure) and per-note expressions as ordinary undoable
 * document edits (CONTRACTS.md §13.2). `SetTrackMpe` is the `mpe` node's (`./mpe.ts`).
 *
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { ExpressionCommand } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";
import { setTrackMpe } from "./mpe";

export function expressionCommand(ctx: ReducerContext, c: ExpressionCommand): void {
  if (c.type === "SetTrackMpe") return setTrackMpe(ctx, c.track, c.mpe);
  fail("Unsupported", `Expression::${c.type} is not implemented yet (midi-expression)`);
}
