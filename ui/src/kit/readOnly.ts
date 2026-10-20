import { useReadOnly } from "@/state/viewer";

/**
 * A control's `disabled`, also on while the app shows a shared project read-only (the static
 * share viewer): knobs, faders, switches, selects and number fields show
 * their values and change nothing. Controls a listener may use (transport, Solo / Mute) are not
 * kit value controls.
 */
export function useDisabled(disabled: boolean | undefined): boolean {
  return useReadOnly() || !!disabled;
}
