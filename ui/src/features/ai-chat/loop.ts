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
export const cappedResult = (n: number) =>
  `Not run: this turn reached its limit of ${n} tool rounds. Tell the user what is left and ask whether to continue.`;

/** Run one turn. Rejects on API errors (auth, rate limit, network); Stop resolves `stopped`. */
export async function runTurn(o: TurnOptions): Promise<TurnEnd> {
  let rounds = 0;
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
    if (step.calls.length === 0) {
      o.session.commit(step);
      return step.stop === "max_tokens" ? { reason: "max_tokens" } : { reason: "end_turn" };
    }
    // A tool input cut off at max_tokens may parse as a valid partial object: never run it.
    if (step.stop === "max_tokens") return { reason: "max_tokens" };

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
    rounds += 1;
  }
}
