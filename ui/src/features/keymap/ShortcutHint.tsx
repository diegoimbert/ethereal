import { useShortcutLabel } from "./store";

/**
 * Action `action`'s current chord, as text that follows the keymap (#574): for copy that
 * names a shortcut ("the command palette (⌘K)"). `fallback` when the action has no chord.
 */
export function ShortcutHint({ action, fallback = "" }: { action: string; fallback?: string }) {
  return <>{useShortcutLabel(action) ?? fallback}</>;
}
