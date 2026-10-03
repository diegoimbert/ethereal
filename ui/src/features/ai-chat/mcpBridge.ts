// Desktop only: the opt-in loopback bridge external AI agents (the `ether-mcp` server) attach
// to. Tauri commands from the agent contract (implemented by `agent-api`).
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export interface BridgeStatus {
  enabled: boolean;
  port: number | null;
  connected_clients: number;
}

/** Injectable for tests. */
export const bridgeIpc = {
  invoke: tauriInvoke as <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>,
};

export function bridgeStatus(): Promise<BridgeStatus> {
  return bridgeIpc.invoke<BridgeStatus>("agent_bridge_status");
}

export async function setBridgeEnabled(enabled: boolean): Promise<BridgeStatus> {
  await bridgeIpc.invoke<unknown>("agent_bridge_set_enabled", { enabled });
  return bridgeStatus();
}

/** The line to register Ethereal with Claude Code (docs/MCP.md). */
export const CLAUDE_MCP_ADD = "claude mcp add ethereal -- ether-mcp";
