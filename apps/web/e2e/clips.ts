// Shared e2e gestures on arrangement clips. Import from here instead of re-deriving the
// gesture in each spec, so a UX change to how clips take the pointer is fixed in one place.
import type { Page } from "@playwright/test";

/**
 * Opens a clip in its editor (piano roll for MIDI, the Warp tab for audio), the way a user
 * does: a double-click on the clip's title bar. Only the title bar (and the edge handles)
 * take the pointer; a clip's body lets presses through to the lane (locate, marquee), so
 * a double-click aimed at the body lands on the lane instead.
 */
export async function openClip(page: Page, clipId: string): Promise<void> {
  await page.locator(`[data-clip-id="${clipId}"] .eth-clip__title`).dblclick();
}

/** Selects a clip the way a user does: a click on its title bar (the body lets clicks through). */
export async function selectClip(page: Page, clipId: string): Promise<void> {
  await page.locator(`[data-clip-id="${clipId}"] .eth-clip__title`).click();
}
