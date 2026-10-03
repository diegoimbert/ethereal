// Undo history panel on the web build (real wasm controller): the History tab lists the
// edits, clicking a step jumps there (undo/redo), checkpoints are named, and edits made
// elsewhere (Undo in the menu, new tracks) show up while the panel is open.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, newProject } from "./ui";

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

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await newProject(page, "History e2e");

  await page.getByRole("button", { name: "History", exact: true }).click();
  const panel = page.locator('[data-feature="undo-history"]');
  const list = panel.getByRole("list", { name: "Undo history" });
  await expect(list.getByRole("listitem")).toHaveCount(1);
  await expect(panel).toContainText("Project opened");

  // Edits made while the panel is open show up (HistoryEvent::Changed).
  await createTrack(page, "Midi");
  await createTrack(page, "Midi");
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

  expect(errors).toEqual([]);
});
