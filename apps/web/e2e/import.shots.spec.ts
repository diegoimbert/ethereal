// Screenshots of the project import for the PR (not part of the regular suite):
// `IMPORT_SHOTS=<dir> npx playwright test import.shots`.
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import { importFixtures } from "./importFixtures";
import { HAS_IMPORTER, NO_IMPORTER } from "./importer";
import { importFromAnotherDaw, projectScreen, resolveLater } from "./ui";

const dir = process.env.IMPORT_SHOTS;
test.skip(!dir, "set IMPORT_SHOTS=<dir> to capture");

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
test.skip(!HAS_IMPORTER, NO_IMPORTER);
const fixtures = HAS_IMPORTER ? importFixtures((p) => readFileSync(resolve(repoRoot, p))) : null!;

/** The launch project screen in `theme` (the app's own theme switch, not the OS scheme). */
async function boot(page: Page, theme: "dark" | "light") {
  await page.goto("/");
  await page.evaluate((t) => {
    document.documentElement.dataset.theme = t;
  }, theme);
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
}

/** Pick `name` with the project menu's "Import from another DAW" `source`; the report dialog. */
async function importFile(page: Page, source: string, name: string, buffer: Buffer, title: string) {
  const chooser = page.waitForEvent("filechooser");
  await importFromAnotherDaw(page, source);
  await (await chooser).setFiles([{ name, mimeType: "application/octet-stream", buffer }]);
  const report = page.getByRole("dialog", { name: `imported “${title}”` });
  await expect(report).toBeVisible({ timeout: 30_000 });
  return report;
}

async function shot(page: Page, name: string) {
  await page.waitForTimeout(600); // enter/exit transitions settle
  await page.screenshot({ path: `${dir}/${name}.png`, animations: "disabled" });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(theme, () => {
    test.use({ viewport: { width: 1440, height: 900 }, colorScheme: theme });

    test(`project screen and import reports (${theme})`, async ({ page }) => {
      await boot(page, theme);
      await shot(page, `import-project-screen-${theme}`);
      let report = await importFile(page, "dawproject…", "Night Drive.dawproject", fixtures.full, "Night Drive");
      await shot(page, `import-report-dawproject-${theme}`);
      await report.getByRole("button", { name: "done" }).click();
      await expect(report).toBeHidden();
      await resolveLater(page);
      await shot(page, `import-opened-${theme}`);
      report = await importFile(page, "reaper…", "Session.rpp", fixtures.rpp, "Session");
      await shot(page, `import-report-reaper-${theme}`);
      await report.getByRole("button", { name: "done" }).click();
      await expect(report).toBeHidden();
      await resolveLater(page);
    });
  });
}
