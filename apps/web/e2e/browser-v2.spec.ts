// The indexed library browser (browser-v2) on the web build, against the real engine
// (WasmTransport → controller Worker → library index): search the index, filter by kind,
// preview a result (tempo-synced), stop it; screenshots of the panel in both themes.
//
// Screenshots go to the test output folder, and also to `$BROWSER_V2_SHOTS/
// browser-v2-shot-{dark,light}.png` when that variable names a directory.
//
// No sleeps: every step waits on UI state.
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { openLibrary, playButton } from "./ui";

const panel = (page: Page) => page.locator('[data-feature="browser"][data-browser="v2"]');
const results = (page: Page) => panel(page).getByRole("list", { name: "Search results" });
const row = (page: Page, name: string) => results(page).getByRole("button", { name, exact: true });

for (const theme of ["dark", "light"] as const) {
  test(`browser v2: search, kind filter and preview (${theme})`, async ({ page }, info) => {
    test.setTimeout(120_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    await page.goto("/");
    await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
    await openLibrary(page);
    await expect(panel(page)).toBeVisible({ timeout: 20_000 });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    // --- Search the index (indexing runs in the background: results arrive on IndexChanged)
    await panel(page).getByRole("searchbox", { name: "Search files" }).fill("loop");
    await expect(row(page, "Chords Loop 120.wav")).toBeVisible({ timeout: 30_000 });
    await expect(row(page, "Kick.wav")).toHaveCount(0);
    await expect(panel(page).getByRole("combobox", { name: "Sort" })).toHaveText(/Relevance/);

    // --- Kind filter ---------------------------------------------------------------------------
    const samples = panel(page).getByRole("group", { name: "Kinds" }).getByRole("button", { name: "Samples", exact: true });
    await samples.click();
    await expect(samples).toHaveAttribute("aria-pressed", "true");
    await expect(row(page, "Chords Loop 120.wav")).toBeVisible();
    await expect(panel(page).getByText(/\d+ items?$/)).toBeVisible();

    // --- Preview (Sync on by default) ---------------------------------------------------------
    await expect(panel(page).getByRole("button", { name: "Sync", exact: true })).toHaveAttribute("aria-pressed", "true");
    await row(page, "Chords Loop 120.wav").click();
    await expect(row(page, "Chords Loop 120.wav")).toHaveAttribute("aria-pressed", "true");

    const shot = await panel(page).screenshot({ timeout: 60_000 });
    await info.attach(`browser-v2-${theme}`, { body: shot, contentType: "image/png" });
    await writeFile(info.outputPath(`browser-v2-shot-${theme}.png`), shot);
    const dir = process.env.BROWSER_V2_SHOTS;
    if (dir) await writeFile(join(dir, `browser-v2-shot-${theme}.png`), shot);

    // --- Stop ----------------------------------------------------------------------------------
    await row(page, "Chords Loop 120.wav").click();
    await expect(row(page, "Chords Loop 120.wav")).toHaveAttribute("aria-pressed", "false");
    await expect(panel(page).getByRole("alert")).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}
