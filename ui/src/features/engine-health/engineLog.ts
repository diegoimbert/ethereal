// Desktop command: open a terminal that follows the engine's log file (`log_file.rs`).
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

/** Injectable for tests. */
export const engineLogIpc = {
  invoke: tauriInvoke as (cmd: string) => Promise<unknown>,
};

export function openEngineLog(): Promise<unknown> {
  return engineLogIpc.invoke("open_engine_log");
}
