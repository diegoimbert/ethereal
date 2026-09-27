/**
 * What the roadmap v2 runtime simulations (`MockMidiLearn`, `MockExports`) may use from the
 * MockTransport. Implemented by `MockTransport`; owned by contracts-2 / base.
 */

import type { Command, Event, Project } from "@/generated";

export interface MockHost {
  /** The live engine-side document (read-only by convention). */
  project(): Project;
  emit(event: Event): void;
  newId(): string;
  /** Apply document commands as one undo step (emits the patch). Throws on failure. */
  applyDocument(commands: Command[], label: string): void;
  /** Run any command as if the UI sent it without a gesture. Throws on failure. */
  execute(command: Command): void;
}
