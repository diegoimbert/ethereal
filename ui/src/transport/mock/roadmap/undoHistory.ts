/**
 * Mock of `History::*` (v0.3, undo-history; CONTRACTS.md §13.8): the undo history as a list
 * (`List` → `History`), jumping to a step (`JumpTo`: an undo/redo run, one patch) and named
 * checkpoints (`SetCheckpoint`). Once a client listed the history, `HistoryEvent::Changed`
 * is pushed after every change of it (the engine throttles to 100 ms; the mock pushes at
 * once). The mock has a single history, so a simulated peer's edits are listed too (the
 * engine lists only this site's own steps).
 */

import type { Event, HistoryCommand, HistoryList, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

/** Longest checkpoint name (characters), like the engine. */
export const MAX_CHECKPOINT_CHARS = 120;

/** What the history panel reads of a MockTransport undo step. */
export interface MockHistoryStep {
  id: number;
  label: string;
  time_ms: number;
  checkpoint: string | null;
}

/** Implemented by `MockTransport` over its undo/redo stacks. */
export interface UndoHistoryHost {
  emit(event: Event): void;
  /** Applied steps, oldest first. */
  undoStack(): readonly MockHistoryStep[];
  /** Undone steps; the last one is redone first. */
  redoStack(): readonly MockHistoryStep[];
  /** Steps were dropped from the front (history limit). */
  truncated(): boolean;
  /** Undo (`n < 0`) or redo (`n > 0`) `|n|` steps, emitting one patch. */
  move(n: number): void;
}

export class MockUndoHistory {
  private listening = false;
  private lastSent = "";

  constructor(private readonly host: UndoHistoryHost) {}

  list(): HistoryList {
    const applied = this.host.undoStack();
    const undone = [...this.host.redoStack()].reverse();
    const step = (s: MockHistoryStep, isUndone: boolean) => ({
      id: s.id,
      label: s.label,
      time_ms: s.time_ms,
      undone: isUndone,
      checkpoint: s.checkpoint,
    });
    return {
      steps: [...applied.map((s) => step(s, false)), ...undone.map((s) => step(s, true))],
      current: applied.at(-1)?.id ?? null,
      truncated: this.host.truncated(),
    };
  }

  /** Call after every change of the history (step recorded, undo/redo, checkpoint, load). */
  changed(): void {
    if (!this.listening) return;
    const history = this.list();
    const json = JSON.stringify(history);
    if (json === this.lastSent) return;
    this.lastSent = json;
    this.host.emit({ type: "History", event: { type: "Changed", history } });
  }

  private reply(): ReplyValue {
    const history = this.list();
    this.listening = true;
    this.lastSent = JSON.stringify(history);
    return { type: "History", history };
  }

  command(c: HistoryCommand): ReplyValue {
    switch (c.type) {
      case "List":
        return this.reply();
      case "JumpTo": {
        const n = this.distanceTo(c.step);
        if (n === null) fail("NotFound", `history step ${c.step} (dropped or unknown)`);
        if (n !== 0) this.host.move(n);
        return this.reply();
      }
      case "SetCheckpoint": {
        const trimmed = c.name?.trim() ?? "";
        if ([...trimmed].length > MAX_CHECKPOINT_CHARS) {
          fail("InvalidArgument", `a checkpoint name is at most ${MAX_CHECKPOINT_CHARS} characters`);
        }
        const step = [...this.host.undoStack(), ...this.host.redoStack()].find((s) => s.id === c.step);
        if (!step) fail("NotFound", `history step ${c.step}`);
        step.checkpoint = trimmed || null;
        this.changed();
        return { type: "Unit" };
      }
    }
  }

  /** Like `ether_model::History::distance_to`: `-n` = undo n, `+n` = redo n, null = unknown. */
  private distanceTo(id: number | null): number | null {
    const undo = this.host.undoStack();
    if (id === null) return -undo.length;
    const i = undo.findIndex((s) => s.id === id);
    if (i >= 0) return -(undo.length - 1 - i);
    const redo = this.host.redoStack();
    const j = redo.findIndex((s) => s.id === id);
    return j >= 0 ? redo.length - j : null;
  }
}
