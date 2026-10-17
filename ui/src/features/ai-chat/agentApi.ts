// The agent API (`Command::Agent`, implemented in the controller's agent tool registry): the
// same tool registry the MCP server exposes. The chat only ever edits the project through
// these two commands.
//
// Every shape here comes from `@/generated` (ts-rs output of ether-protocol, checked for
// staleness in CI), with no casts: a protocol change to the agent domain fails the UI typecheck.
import type { AgentCommand, AgentToolSpec, Command, ReplyValue } from "@/generated";
import type { EngineTransport } from "@/transport";

export type { AgentCommand, AgentToolSpec };

/** The replies to agent commands. */
export type AgentReply = Extract<ReplyValue, { type: "AgentTools" | "AgentToolResult" }>;

/** What a tool call gave back to the model. */
export type ToolOutcome = Omit<Extract<AgentReply, { type: "AgentToolResult" }>, "type">;

export function agentCommand(command: AgentCommand): Command {
  return { domain: "Agent", command };
}

function isReply<T extends AgentReply["type"]>(value: ReplyValue, type: T): value is Extract<ReplyValue, { type: T }> {
  return value.type === type;
}

function reply<T extends AgentReply["type"]>(value: ReplyValue, type: T): Extract<ReplyValue, { type: T }> {
  if (!isReply(value, type)) throw new Error(`unexpected reply to agent command: ${value.type}`);
  return value;
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
