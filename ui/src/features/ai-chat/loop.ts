// eth-allow-case-file: tool results for the model, not the UI.
// The model loop of one chat turn, for any provider: stream a response, run each tool call
// through the agent API, send the results back, until the model ends its turn. The provider
// specifics (wire format, history shape) live behind `ChatSession` (providers/).
import type { ToolOutcome } from "./agentApi";
import { isAbortError, type ChatSession, type Connection, type LoopEvent, type Step, type ToolResult } from "./providers";

export type { LoopEvent };

export type TurnEnd =
  | { reason: "end_turn" }
  | { reason: "stopped" }
  /** The tool-iteration cap was hit; the pending calls were answered as not run. */
  | { reason: "capped"; iterations: number }
  | { reason: "refusal" }
  /** Responses kept getting cut off at the output limit without progress (after continuing). */
  | { reason: "max_tokens" };

export interface TurnOptions {
  /** The conversation, whose last message is the user's. Appended to, never edited. */
  session: ChatSession;
  conn: Connection;
  /** Most tool rounds this turn. */
  maxIterations: number;
  signal: AbortSignal;
  callTool(name: string, input: unknown): Promise<ToolOutcome>;
  onEvent(event: LoopEvent): void;
}

export const STOPPED_RESULT = "Not run: the user pressed Stop.";
export const cappedResult = (n: number) => `Not run: this turn reached its limit of ${n} tool rounds. Tell the user what is left and ask whether to continue.`;

/** A tool call whose input was cut off at the output limit (shown on its chip). */
export const CUT_RESULT = "Not run: the response was cut off (output limit) before this call was complete.";

/** Cut-off responses in a row that made no progress (no complete call) before the turn gives up. */
export const MAX_FRUITLESS_CUTS = 2;

/** What the model is told after a cut-off response, so it continues instead of failing. */
export function continueNote(cut: ReadonlyArray<{ name: string }>): string {
  const lost =
    cut.length > 0
      ? ` The call${cut.length === 1 ? "" : "s"} ${cut.map((c) => c.name).join(", ")} ${cut.length === 1 ? "was" : "were"} cut off before ${cut.length === 1 ? "its" : "their"} input was complete and NOT run; send ${cut.length === 1 ? "it" : "them"} again.`
      : "";
  return (
    `[Your last response was cut off: it reached the output limit.${lost} ` +
    "Continue from where you stopped, without repeating what already ran. Split big edits into several smaller tool calls " +
    "(e.g. a few bars of notes per call) and keep your text short.]"
  );
}

/** Run one turn. Rejects on API errors (auth, rate limit, network); Stop resolves `stopped`. */
export async function runTurn(o: TurnOptions): Promise<TurnEnd> {
  let rounds = 0;
  let fruitless = 0;
  for (;;) {
    if (o.signal.aborted) return { reason: "stopped" };
    let step: Step;
    try {
      step = await o.session.step(o.conn, o.signal, o.onEvent);
    } catch (e) {
      if (isAbortError(e) || o.signal.aborted) return { reason: "stopped" };
      throw e;
    }
    if (step.stop === "refusal") return { reason: "refusal" };
    const truncated = step.stop === "max_tokens";
    // A tool input cut off at the output limit may parse as a valid partial object: never run
    // it (the adapters move it to `cut`). The complete calls before it run as usual, and the
    // model is asked to continue: a long batch of edits goes on over several responses.
    for (const c of step.cut ?? []) o.onEvent({ type: "tool_result", id: c.id, content: CUT_RESULT, is_error: true });
    if (step.calls.length === 0) {
      o.session.commit(step);
      if (!truncated) return { reason: "end_turn" };
      fruitless += 1;
      if (fruitless > MAX_FRUITLESS_CUTS || rounds >= o.maxIterations) return { reason: "max_tokens" };
      o.session.addNote(continueNote(step.cut ?? []));
      rounds += 1;
      continue;
    }
    fruitless = 0;

    o.session.commit(step);
    const capped = rounds >= o.maxIterations;
    const results: ToolResult[] = [];
    // One call at a time, in order (also right for providers without parallel tool calls).
    for (const call of step.calls) {
      let outcome: ToolOutcome;
      if (capped) outcome = { content: cappedResult(o.maxIterations), is_error: true };
      else if (o.signal.aborted) outcome = { content: STOPPED_RESULT, is_error: true };
      else if (call.invalid !== undefined) outcome = { content: call.invalid, is_error: true };
      else outcome = await o.callTool(call.name, call.input);
      results.push({ id: call.id, name: call.name, ...outcome });
      o.onEvent({ type: "tool_result", id: call.id, ...outcome });
    }
    // Every call gets its result (the history stays valid for the next turn).
    o.session.addToolResults(results);
    if (capped) return { reason: "capped", iterations: o.maxIterations };
    if (o.signal.aborted) return { reason: "stopped" };
    if (truncated) o.session.addNote(continueNote(step.cut ?? []));
    rounds += 1;
  }
}
