/**
 * What the roadmap v2 runtime simulations (`MockMidiLearn`, `MockExports`) may use from the
 * MockTransport. Implemented by `MockTransport`; owned by contracts-2 / base.
 */

import type { Command, Event, Project } from "@/generated";
import type { Tx } from "../tx";

export interface MockHost {
  /** The live engine-side document (read-only by convention). */
  project(): Project;
  emit(event: Event): void;
  newId(): string;
  /** Apply document commands as one undo step (emits the patch). Throws on failure. */
  applyDocument(commands: Command[], label: string): void;
  /** Run any command as if the UI sent it without a gesture. Throws on failure. */
  execute(command: Command): void;
  /**
   * Apply a transaction and emit its patch WITHOUT an undo step (collab-social: chat is a
   * journal, never undone; docs/COLLAB.md §12.1). Rolled back if `body` throws. Optional so
   * other nodes' test hosts need not stub it (`MockTransport` implements it).
   */
  applyUntracked?(body: (tx: Tx) => void): void;
}
