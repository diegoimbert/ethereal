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
  page.evaluate(
    () =>
      Object.values((window as unknown as { __ether: Handle }).__ether.state().project?.tracks ?? {}).filter(
        (t) => t.kind === "Midi",
      ).length,
  );

test("History tab: list, jump, checkpoints", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, "History e2e");

  const panel = page.locator('[data-feature="undo-history"]');
  const list = panel.getByRole("list", { name: "Undo history" });
  const historyButton = page.getByRole("button", { name: "History", exact: true });
  await historyButton.click();
  await expect(list.getByRole("listitem")).toHaveCount(1);
  await expect(panel).toContainText("Project opened");
  await historyButton.click();

  // (The floating pane covers the arrangement: edit with it closed.)
  await createTrack(page, "Midi");
  await createTrack(page, "Midi");
  await historyButton.click();
  await expect(list.getByRole("listitem")).toHaveCount(3);
  await expect(panel).toContainText("2 steps");

  // Back to before everything: both tracks go.
  await list.getByRole("button", { name: /Project opened/ }).click();
  await expect.poll(() => trackCount(page)).toBe(0);
  await expect(panel).toContainText("2 undone");

  // Forward to the first step: one track back.
  const rows = list.getByRole("listitem");
  await rows.nth(1).locator(".eth-history__jump").click();
  await expect.poll(() => trackCount(page)).toBe(1);
  await expect(rows.nth(1).locator(".eth-history__jump")).toHaveAttribute("aria-current", "step");

  // Name it.
  await rows.nth(1).hover();
  await rows.nth(1).getByRole("button", { name: "Name checkpoint" }).click();
  await rows.nth(1).getByRole("textbox").fill("One track");
  await rows.nth(1).getByRole("textbox").press("Enter");
  await expect(rows.nth(1)).toContainText("One track");

  // Undo from the keyboard: the panel follows.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => trackCount(page)).toBe(0);
  await expect(list.locator('[aria-current="step"]')).toContainText("Project opened");
  // The checkpoint stays on its (now undone) step.
  await expect(rows.nth(1)).toContainText("One track");

  // Redo from the keyboard while the panel is open (HistoryEvent::Changed).
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(() => trackCount(page)).toBe(1);
  await expect(rows.nth(1).locator(".eth-history__jump")).toHaveAttribute("aria-current", "step");

  expect(errors).toEqual([]);
});
