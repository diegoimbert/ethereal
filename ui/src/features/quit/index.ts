// Desktop quit confirmation and ⌘W routing (`apps/desktop/src-tauri/src/quit.rs`).
export { CLOSE_REQUESTED_EVENT, QUIT_REQUESTED_EVENT, quitBridge, setQuitBridge, tauriQuitBridge, type QuitBridge } from "./bridge";
export { QuitRoot } from "./QuitRoot";
export { useQuitConfirm } from "./store";
