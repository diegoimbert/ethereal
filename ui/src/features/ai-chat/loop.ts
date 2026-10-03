// The model loop of one chat turn: stream a Messages API response, run each `tool_use`
// through the agent API, send the `tool_result`s back, until the model ends its turn.
//
// History is append-only (thinking blocks and fallback blocks are echoed back unchanged),
// so prompt caching and preserved thinking keep working across turns.
import Anthropic from "@anthropic-ai/sdk";
import type {
  BetaContentBlock,
  BetaMessage,
  BetaMessageParam,
  BetaTextBlockParam,
  BetaTool,
  BetaToolResultBlockParam,
  BetaToolUseBlock,
  BetaMessageStreamParams,
} from "@anthropic-ai/sdk/resources/beta/messages/messages";
import type { AgentToolSpec, ToolOutcome } from "./agentApi";
import type { ModelId } from "./settings";

export type LoopEvent =
  /** A new assistant text segment starts (after a tool call, a new bubble). */
  | { type: "text_start" }
  | { type: "text"; delta: string }
  | { type: "tool_call"; id: string; name: string; input: unknown }
  | { type: "tool_result"; id: string; content: string; is_error: boolean };

export type TurnEnd =
  | { reason: "end_turn" }
  | { reason: "stopped" }
  /** The tool-iteration cap was hit; the pending calls were answered as not run. */
  | { reason: "capped"; iterations: number }
  | { reason: "refusal" }
  | { reason: "max_tokens" };

export interface TurnOptions {
  client: Anthropic;
  model: ModelId;
  system: BetaTextBlockParam[];
  tools: BetaTool[];
  /** The conversation so far, ending with the user's message. Appended to, never edited. */
  messages: BetaMessageParam[];
  /** Most tool rounds this turn. */
  maxIterations: number;
  signal: AbortSignal;
  callTool(name: string, input: unknown): Promise<ToolOutcome>;
  onEvent(event: LoopEvent): void;
}

/** Output budget per response (streamed, so no request timeout concerns). */
export const MAX_TOKENS = 32_000;

export const STOPPED_RESULT = "Not run: the user pressed Stop.";
export const cappedResult = (n: number) =>
  `Not run: this turn reached its limit of ${n} tool rounds. Tell the user what is left and ask whether to continue.`;

/** The registry's tools as Messages API tool definitions (`input_schema` parsed from JSON text). */
export function toApiTools(specs: ReadonlyArray<AgentToolSpec>): BetaTool[] {
  return specs.map((s) => {
    let schema: BetaTool["input_schema"];
    try {
      const parsed = JSON.parse(s.input_schema) as Record<string, unknown>;
      schema = { ...parsed, type: "object" } as BetaTool["input_schema"];
    } catch {
      schema = { type: "object" };
    }
    return { name: s.name, description: s.description, input_schema: schema };
  });
}

/** Request parameters for `model` (shared by every request of a turn). */
export function requestParams(o: Pick<TurnOptions, "model" | "system" | "tools" | "messages">): BetaMessageStreamParams {
  const params: BetaMessageStreamParams = {
    model: o.model,
    max_tokens: MAX_TOKENS,
    system: o.system,
    tools: o.tools,
    messages: o.messages,
    // Caches the longest stable prefix (tools, system, history) between requests.
    cache_control: { type: "ephemeral" },
  };
  // Server-side fallback when a safety classifier declines (Sonnet 5.5 / Opus 5.5).
  if (o.model === "claude-sonnet-5-5" || o.model === "claude-opus-5-5") {
    params.betas = ["server-side-fallback-2026-07-01"];
    params.fallbacks = "default";
  }
  return params;
}

const isAbort = (e: unknown) => e instanceof Anthropic.APIUserAbortError || (e instanceof Error && e.name === "AbortError");

/** Run one turn. Rejects on API errors (auth, rate limit, network); Stop resolves `stopped`. */
export async function runTurn(o: TurnOptions): Promise<TurnEnd> {
  let rounds = 0;
  for (;;) {
    if (o.signal.aborted) return { reason: "stopped" };
    let message: BetaMessage;
    try {
      message = await streamOnce(o);
    } catch (e) {
      if (isAbort(e) || o.signal.aborted) return { reason: "stopped" };
      throw e;
    }
    if (message.stop_reason === "refusal") return { reason: "refusal" };
    const calls = message.content.filter((b): b is BetaToolUseBlock => b.type === "tool_use");
    if (calls.length === 0) {
      o.messages.push({ role: "assistant", content: message.content as BetaContentBlock[] });
      return message.stop_reason === "max_tokens" ? { reason: "max_tokens" } : { reason: "end_turn" };
    }
    // A tool input cut off at max_tokens may parse as a valid partial object: never run it.
    if (message.stop_reason === "max_tokens") return { reason: "max_tokens" };

    o.messages.push({ role: "assistant", content: message.content as BetaContentBlock[] });
    const capped = rounds >= o.maxIterations;
    const results: BetaToolResultBlockParam[] = [];
    for (const call of calls) {
      let outcome: ToolOutcome;
      if (capped) outcome = { content: cappedResult(o.maxIterations), is_error: true };
      else if (o.signal.aborted) outcome = { content: STOPPED_RESULT, is_error: true };
      else outcome = await o.callTool(call.name, call.input);
      results.push({ type: "tool_result", tool_use_id: call.id, content: outcome.content, ...(outcome.is_error ? { is_error: true } : {}) });
      o.onEvent({ type: "tool_result", id: call.id, ...outcome });
    }
    // All results of one assistant message go back in ONE user message.
    o.messages.push({ role: "user", content: results });
    if (capped) return { reason: "capped", iterations: o.maxIterations };
    if (o.signal.aborted) return { reason: "stopped" };
    rounds += 1;
  }
}

async function streamOnce(o: TurnOptions): Promise<BetaMessage> {
  const stream = o.client.beta.messages.stream(requestParams(o), { signal: o.signal });
  for await (const event of stream) {
    if (event.type === "content_block_start") {
      if (event.content_block.type === "text") o.onEvent({ type: "text_start" });
    } else if (event.type === "content_block_delta") {
      if (event.delta.type === "text_delta") o.onEvent({ type: "text", delta: event.delta.text });
    } else if (event.type === "content_block_stop") {
      const block = stream.currentMessage?.content[event.index];
      if (block?.type === "tool_use") o.onEvent({ type: "tool_call", id: block.id, name: block.name, input: block.input });
    }
  }
  return stream.finalMessage();
}
