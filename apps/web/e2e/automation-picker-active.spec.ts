// automation-picker-active: the parameter picker's "Active" toggle (only automated
// parameters), shared by all tracks and kept across reloads, ignored where nothing is automated.
import { expect, test, type Page } from "@playwright/test";
import { createTrack, launch, newProject, headerButton } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

async function openPicker(page: Page, trackName: string) {
  const trigger = page
    .locator(`[data-automation-track]`)
    .filter({ has: page.getByLabel(`automation of ${trackName}`) })
    .getByRole("combobox", { name: "show parameter", exact: true });
  await trigger.click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  const list = page.locator(`[id="${await trigger.getAttribute("aria-controls")}"]`);
  return { trigger, list, toggle: page.getByRole("button", { name: "show only automated parameters" }) };
}

test("the picker's Active toggle filters to automated params, persists, and is ignored without any", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await newProject(page, `Picker ${Date.now()}`);
  const a = await createTrack(page, "Midi");
  const b = await createTrack(page, "Midi");

  // Track A: automate Transpose, then hide its lane so it is automated but listed in the picker.
  await (await headerButton(page, a.name, `Show automation of ${a.name}`)).click();
  const synthParam = page.locator(`[data-automation-track="${a.id}"]`).getByRole("combobox", { name: "show parameter", exact: true });
  await synthParam.click();
  const listId = await synthParam.getAttribute("aria-controls");
  await page.locator(`[id="${listId}"] [role="option"]`).filter({ hasText: "transpose" }).first().click();
  const transpose = page.getByRole("group", { name: /transpose automation/ });
  const svg = transpose.getByTestId("automation-lane-svg");
  await expect(svg).toBeVisible();
  await svg.dblclick({ position: { x: 80, y: 20 } });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          Object.keys((window as unknown as { __ether: { state(): { project: { automation_points: object } } } }).__ether.state().project.automation_points)
            .length,
      ),
    )
    .toBe(1);
  await page.getByRole("button", { name: "hide lane" }).last().click();
  await expect(transpose).toBeHidden();

  // Active shows only the automated parameter.
  let picker = await openPicker(page, a.name);
  const all = await picker.list.getByRole("option").count();
  expect(all).toBeGreaterThan(1);
  await expect(picker.toggle).toBeEnabled();
  await picker.toggle.click();
  await expect(picker.toggle).toHaveAttribute("aria-pressed", "true");
  await expect(picker.list.getByRole("option")).toHaveCount(1);
  await expect(picker.list.getByRole("option").first()).toContainText("transpose");
  await page.keyboard.press("Escape");

  // Another track without automation: full list, toggle off and disabled (preference kept).
  await (await headerButton(page, b.name, `Show automation of ${b.name}`)).click();
  picker = await openPicker(page, b.name);
  await expect(picker.toggle).toBeDisabled();
  await expect(picker.toggle).toHaveAttribute("aria-pressed", "false");
  expect(await picker.list.getByRole("option").count()).toBeGreaterThan(1);
  await page.keyboard.press("Escape");
  expect(await page.evaluate(() => localStorage.getItem("eth.automation.pickerActiveOnly"))).toBe("1");

  // Reload: the preference survives.
  await page.reload();
  expect(await page.evaluate(() => localStorage.getItem("eth.automation.pickerActiveOnly"))).toBe("1");

  expect(errors).toEqual([]);
});
