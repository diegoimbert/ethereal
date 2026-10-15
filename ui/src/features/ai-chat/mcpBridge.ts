// Desktop only: the opt-in loopback bridge external AI agents (the `ether-mcp` server) attach
// to. Tauri commands from the agent contract (implemented by `agent-api`).
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export interface BridgeStatus {
  enabled: boolean;
  port: number | null;
  connected_clients: number;
  /** The `ether-mcp` shipped next to the app (release builds); null in dev builds and AppImages. */
  mcp_path?: string | null;
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

/** The line to register Ethereal with Claude Code (docs/MCP.md) when the binary's path is unknown. */
export const CLAUDE_MCP_ADD = "claude mcp add ethereal -- ether-mcp";

/** The line to register Ethereal with Claude Code, with the bundled `ether-mcp` when there is one. */
export function claudeMcpAdd(mcpPath: string | null | undefined): string {
  if (!mcpPath) return CLAUDE_MCP_ADD;
  return `claude mcp add ethereal -- ${shellQuote(mcpPath)}`;
}

/** Quotes a path for the user's shell: cmd/PowerShell for a Windows path, POSIX sh otherwise. */
function shellQuote(path: string): string {
  if (/^[A-Za-z]:\\/.test(path)) return /\s/.test(path) ? `"${path}"` : path;
  return /^[\w./+-]+$/.test(path) ? path : `'${path.replace(/'/g, "'\\''")}'`;
}
