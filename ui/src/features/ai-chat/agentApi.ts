// The agent API (Command::Agent, pinned in the agent contract; implemented in the controller by
// `agent-api`): the same tool registry the MCP server exposes. The chat only ever edits the
// project through these two commands.
//
// TODO(agent-api): these types mirror the contract by hand until `ui/src/generated` has
// `AgentCommand` / `AgentToolSpec` and the `AgentTools` / `AgentToolResult` reply variants;
// then import them from "@/generated" and drop the casts below.
import type { Command, ReplyValue } from "@/generated";
import type { EngineTransport } from "@/transport";

/** A tool of the registry. `input_schema` is a JSON Schema object, as JSON text. */
export interface AgentToolSpec {
  name: string;
  description: string;
  input_schema: string;
}

export type AgentCommand = { type: "ListTools" } | { type: "CallTool"; name: string; input: string };

export type AgentReply =
  | { type: "AgentTools"; tools: AgentToolSpec[] }
  | { type: "AgentToolResult"; content: string; is_error: boolean };

/** What a tool call gave back to the model. */
export interface ToolOutcome {
  content: string;
  is_error: boolean;
}

/** `Command::Agent(command)`, typed until the generated `Command` has the domain. */
export function agentCommand(command: AgentCommand): Command {
  return { domain: "Agent", command } as unknown as Command;
}

function reply<T extends AgentReply["type"]>(value: ReplyValue, type: T): Extract<AgentReply, { type: T }> {
  const v = value as unknown as AgentReply;
  if (v.type !== type) throw new Error(`unexpected reply to Agent command: ${(value as { type: string }).type}`);
  return v as Extract<AgentReply, { type: T }>;
}

export async function listTools(transport: EngineTransport): Promise<AgentToolSpec[]> {
  return reply(await transport.send(agentCommand({ type: "ListTools" })), "AgentTools").tools;
}

/**
 * Run one tool. A bad name or input comes back as an `is_error` result (the model must see
 * it); a failed command (engine gone, ...) is turned into one too, so the loop never breaks
 * on a tool.
 */
export async function callTool(transport: EngineTransport, name: string, input: unknown): Promise<ToolOutcome> {
  try {
    const r = reply(await transport.send(agentCommand({ type: "CallTool", name, input: JSON.stringify(input ?? {}) })), "AgentToolResult");
    return { content: r.content, is_error: r.is_error };
  } catch (e) {
    return { content: e instanceof Error ? e.message : String(e), is_error: true };
  }
}
