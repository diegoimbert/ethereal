// The provider abstraction: one interface for "stream a chat response with tool calls", so
// the tool loop (loop.ts), Stop, the iteration cap and the chips never depend on a provider.
//
// Each adapter keeps its own native history (Anthropic content blocks with thinking echoed back
// unchanged; OpenAI-style messages), behind a `ChatSession`.
import type { AgentToolSpec } from "../agentApi";

export type LoopEvent =
  /** A new assistant text segment starts (after a tool call, a new bubble). */
  | { type: "text_start" }
  | { type: "text"; delta: string }
  | { type: "tool_call"; id: string; name: string; input: unknown }
  | { type: "tool_result"; id: string; content: string; is_error: boolean };

/** A tool, provider-neutral: `parameters` is the JSON Schema object of its input. */
export interface ToolDef {
  name: string;
  description: string;
  parameters: Record<string, unknown> & { type: "object" };
}

/** One tool call the model asked for. `invalid` = its input couldn't be parsed (never run). */
export interface ToolCall {
  id: string;
  name: string;
  input: unknown;
  invalid?: string;
}

export interface ToolResult {
  id: string;
  name: string;
  content: string;
  is_error: boolean;
}

export type StepStop = "end_turn" | "tool_use" | "max_tokens" | "refusal";

/** One streamed response. `raw` is the adapter's native assistant message (for `commit`). */
export interface Step {
  calls: ToolCall[];
  stop: StepStop;
  raw: unknown;
}

/** Where and how to reach the provider for one request. */
export interface Connection {
  baseUrl: string;
  /** `null` for keyless local servers. */
  apiKey: string | null;
  model: string;
}

export interface ChatSession {
  /** The user's message: the parts are joined (or sent as blocks) in order. */
  addUser(parts: string[]): void;
  /**
   * Stream one assistant response for the history so far, emitting `text_start`, `text` and
   * (once its input is complete) `tool_call` events. Leaves the history untouched: a stopped
   * or cut-off response is never added. Rejects with a `ProviderError`; an abort rejects
   * with an `AbortError`.
   */
  step(conn: Connection, signal: AbortSignal, onEvent: (e: LoopEvent) => void): Promise<Step>;
  /** Append the step's assistant message to the history. */
  commit(step: Step): void;
  /** Answer every call of the last committed step (in order). */
  addToolResults(results: ToolResult[]): void;
  /** The native history (tests). */
  readonly history: ReadonlyArray<unknown>;
}

export type ProviderErrorKind =
  | "auth"
  | "permission"
  | "rate_limit"
  | "connection"
  | "cors"
  | "not_found"
  | "no_tools"
  | "server"
  | "other";

/** A failed request, with a user-facing message (never contains the key). */
export class ProviderError extends Error {
  constructor(
    readonly kind: ProviderErrorKind,
    message: string,
    readonly status?: number,
  ) {
    super(message);
    this.name = "ProviderError";
  }
  /** The key was refused: the panel shows the key setup again. */
  get keyRejected(): boolean {
    return this.kind === "auth";
  }
}

export const isAbortError = (e: unknown): boolean => e instanceof Error && (e.name === "AbortError" || e.name === "APIUserAbortError");

/** The registry's tools, provider-neutral (`input_schema` parsed from JSON text; bad schemas become `{type: "object"}`). */
export function toolDefs(specs: ReadonlyArray<AgentToolSpec>): ToolDef[] {
  return specs.map((s) => {
    let parameters: ToolDef["parameters"];
    try {
      const parsed = JSON.parse(s.input_schema) as unknown;
      parameters =
        parsed && typeof parsed === "object" && !Array.isArray(parsed)
          ? { ...(parsed as Record<string, unknown>), type: "object" }
          : { type: "object" };
    } catch {
      parameters = { type: "object" };
    }
    return { name: s.name, description: s.description, parameters };
  });
}

/** `https://api.openai.com/v1` → `api.openai.com` (for messages). */
export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
