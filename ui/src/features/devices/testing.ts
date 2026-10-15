/** Test helpers for the device picker (import in tests only). */
import { fireEvent, screen, within } from "@testing-library/react";

/** Pick a device like a user: open the `DevicePicker` (its trigger button), click the row named `name`. */
export async function pickDevice(trigger: HTMLElement, name: string | RegExp): Promise<void> {
  fireEvent.click(trigger);
  const popover = await screen.findByRole("dialog", { name: trigger.getAttribute("aria-label") ?? "" });
  fireEvent.click(await within(popover).findByRole("button", { name }));
}
