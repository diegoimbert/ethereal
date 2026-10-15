/**
 * Test helpers for kit components (import from "@/kit/testing" in tests only).
 */
import { fireEvent, screen, within } from "@testing-library/react";

/**
 * Choose an option of a kit `Select` like a user: open it, click the option. `option` is
 * the option's label (string or RegExp) or `{ value }` for its value.
 */
export function pickOption(select: HTMLElement, option: string | RegExp | { value: string }): void {
  fireEvent.click(select);
  // The list this trigger controls (another one may still be animating out).
  const id = select.getAttribute("aria-controls");
  const listbox = (id && document.getElementById(id)) || screen.getByRole("listbox");
  const el =
    typeof option === "object" && "value" in option
      ? listbox.querySelector<HTMLElement>(`[role="option"][data-value="${CSS.escape(option.value)}"]`)
      : within(listbox).getByRole("option", { name: option });
  if (!el) throw new Error(`no option ${JSON.stringify(option)}`);
  fireEvent.click(el);
}
