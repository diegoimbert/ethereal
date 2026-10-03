// Screenshots of the base-114 project/collab quick wins for the PR (not part of the regular
// suite): `PROJECT_SHOTS=<dir> npx playwright test project-actions.shots`.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { ProjectSummary } from "@/generated";
import { newProject, projectScreen } from "./ui";

const dir = process.env.PROJECT_SHOTS;
test.skip(!dir, "set PROJECT_SHOTS=<dir> to capture");
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `shots-${Math.random().toString(36).slice(2)}`;
let relay: ChildProcess | null = null;
let relayUrl = "";

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const out = execFileSync("cargo", ["build", "-p", "ether-collab", "--bin", "ether-collab-relay", "--message-format=json"], {
    cwd: repoRoot,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  const bin = out
    .split("\n")
    .filter((l) => l.startsWith("{"))
    .map((l) => JSON.parse(l) as { reason?: string; target?: { name: string }; executable?: string | null })
    .find((m) => m.reason === "compiler-artifact" && m.target?.name === "ether-collab-relay" && m.executable)!.executable!;
  const child = spawn(bin, ["--port", "0", "--token", TOKEN], { stdio: ["ignore", "pipe", "inherit"] });
  relay = child;
  relayUrl = await new Promise<string>((ok) => {
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /listening on (ws:\/\/\S+)/.exec(buf);
      if (m) ok(m[1]!);
    });
  });
});

test.afterAll(() => relay?.kill());

const shot = (page: Page, name: string) => page.screenshot({ path: `${dir}/${name}.png` });

const projects = (page: Page) =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { projects: ProjectSummary[] } } }).__ether.state().projects);

async function boot(page: Page) {
  await page.goto("/");
  // base-131: the app launches with no project, on the project screen.
  await expect(projectScreen(page)).toBeVisible({ timeout: 30_000 });
}

test("project screen, toast, palette, collab dialog, leave warning", async ({ page }) => {
  test.setTimeout(180_000);
  await boot(page);
  await newProject(page, "Night drive");
  await newProject(page, "Beat sketch (local copy)");
  await newProject(page, "Beat sketch");
  await newProject(page, "Late jam");
  await newProject(page, "Chord study");
  // "Late jam" was used in a session (normally recorded while online).
  const jam = (await projects(page)).find((p) => p.name === "Late jam")!;
  await page.evaluate((id) => localStorage.setItem("eth-collab-projects", JSON.stringify({ [id]: "late-jam" })), jam.id);
  await boot(page);

  // 1. Project screen: the open project's actions and the recents badges.
  await page.getByRole("button", { name: "Projects" }).click();
  const screen = page.getByRole("dialog", { name: "Projects" });
  await expect(screen.getByRole("button", { name: /^Open Late jam/ })).toBeVisible();
  await page.waitForTimeout(300);
  await shot(page, "01-project-screen-dark");

  // 2. A notification toast (Duplicate's confirmation; engine notifications use the same toasts).
  await screen.getByRole("group", { name: "Project actions" }).getByRole("button", { name: "Duplicate" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Duplicated as" })).toBeVisible();
  await page.waitForTimeout(300);
  await shot(page, "02-toast-dark");
  await screen.getByRole("button", { name: "Continue" }).click();

  // 3. Palette: project and collab commands.
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByRole("combobox", { name: "Search commands" }).fill("project");
  await page.waitForTimeout(300);
  await shot(page, "03-palette-project-dark");
  await page.getByRole("combobox", { name: "Search commands" }).fill("collab");
  await page.waitForTimeout(200);
  await shot(page, "04-palette-collab-dark");
  await page.keyboard.press("Enter");

  // 4. Collab dialog: inline session-name error and the remembered token.
  await page.getByLabel("Relay address").fill(relayUrl);
  await page.getByLabel("Session").fill("late jam!");
  await page.getByLabel("Your name").fill("Ada");
  await page.getByLabel("Token").fill(TOKEN);
  await page.waitForTimeout(300);
  await shot(page, "05-collab-dialog-dark");
  await page.getByLabel("Session").fill("late-jam");
  await page.getByRole("button", { name: "Join" }).click();
  await expect(page.getByTestId("collab-button")).toHaveText("● late-jam", { timeout: 20_000 });

  // 5. Leave warning.
  await page.getByRole("button", { name: "Projects" }).click();
  await screen.getByRole("button", { name: /^Open Night drive/ }).click();
  await expect(page.getByRole("dialog", { name: "Leave the “late-jam” session?" })).toBeVisible();
  await page.waitForTimeout(300);
  await shot(page, "06-leave-warning-dark");
  await page.getByRole("button", { name: "Cancel" }).click();

  // 6. Light theme: project screen.
  await page.getByRole("button", { name: "Continue" }).click();
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "light";
    localStorage.setItem("eth-theme", "light");
  });
  await page.getByRole("button", { name: "Projects" }).click();
  await screen.getByRole("group", { name: "Project actions" }).getByRole("button", { name: "Export…" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Exported" })).toBeVisible();
  await page.waitForTimeout(300);
  await shot(page, "07-project-screen-light");
});
