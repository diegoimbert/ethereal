// Undo history panel on the web build (real wasm controller): the History tab lists the
// edits, clicking a step jumps there (undo/redo), checkpoints are named, and undo/redo from
// the keyboard show up while the panel is open.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, launch } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const trackCount = (page: Page) =>
  page.evaluate(() => Object.values((window as unknown as { __ether: Handle }).__ether.state().project?.tracks ?? {}).filter((t) => t.kind === "Midi").length);

test("History tab: list, jump, checkpoints", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, "History e2e");

  const panel = page.locator('[data-feature="undo-history"]');
  const list = panel.getByRole("list", { name: "undo history" });
  const rows = list.getByRole("listitem");
  const historyButton = page.getByRole("button", { name: "history", exact: true });
  await historyButton.click();
  // A new project is named after the date and `launch` renames it in place: a Rename step.
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText("project opened");
  await expect(rows.nth(1)).toContainText("rename project");
  await historyButton.click();

  // (The floating pane covers the arrangement: edit with it closed.)
  await createTrack(page, "Midi");
  await createTrack(page, "Midi");
  await historyButton.click();
  await expect(rows).toHaveCount(4);
  await expect(panel).toContainText("3 steps");

  // Back to before the tracks (the rename stays): both tracks go.
  await rows.nth(1).locator(".eth-history__jump").click();
  await expect.poll(() => trackCount(page)).toBe(0);
  await expect(panel).toContainText("2 undone");

  // Forward to the first track: one track back.
  await rows.nth(2).locator(".eth-history__jump").click();
  await expect.poll(() => trackCount(page)).toBe(1);
  await expect(rows.nth(2).locator(".eth-history__jump")).toHaveAttribute("aria-current", "step");

  // Name it.
  await rows.nth(2).hover();
  await rows.nth(2).getByRole("button", { name: "name checkpoint" }).click();
  await rows.nth(2).getByRole("textbox").fill("One track");
  await rows.nth(2).getByRole("textbox").press("Enter");
  await expect(rows.nth(2)).toContainText("One track");

  // Undo from the keyboard: the panel follows.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => trackCount(page)).toBe(0);
  await expect(list.locator('[aria-current="step"]')).toContainText("rename project");
  // The checkpoint stays on its (now undone) step.
  await expect(rows.nth(2)).toContainText("One track");

  // Redo from the keyboard while the panel is open (HistoryEvent::Changed).
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(() => trackCount(page)).toBe(1);
  await expect(rows.nth(2).locator(".eth-history__jump")).toHaveAttribute("aria-current", "step");

  expect(errors).toEqual([]);
});
