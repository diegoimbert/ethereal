// One tab owns the project store. A second tab of the same browser starts no engine and
// says so ("ethereal is open in another tab"); it never deletes the first tab's draft. "use
// here" moves the editor: the first tab saves, stops, and shows "ethereal moved to another tab".
import { expect, test, type Page } from "@playwright/test";
import type { Project, ProjectSummary } from "@/generated";
import { createTrack, projectScreen } from "./ui";

interface Handle {
  state(): { project: Project | null; projects: ProjectSummary[]; dirty: boolean };
}

const state = (page: Page) =>
  page.evaluate(() => {
    const s = (window as unknown as { __ether?: Handle }).__ether?.state();
    return {
      id: s?.project?.id ?? null,
      tracks: s?.project ? Object.keys(s.project.tracks).length : 0,
      dirty: s?.dirty ?? false,
      projects: s?.projects.map((p) => p.id) ?? [],
    };
  });
const engineStarted = (page: Page) => page.evaluate(() => (window as unknown as { __etherEngine?: unknown }).__etherEngine !== undefined);

test("a second tab starts no engine and keeps the first tab's draft; use here moves the editor", async ({ context, page: first }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  first.on("pageerror", (e) => errors.push(e.message));

  await first.goto("/");
  await expect(projectScreen(first)).toBeVisible({ timeout: 30_000 });
  // Launch opened an untouched draft behind the project screen.
  await expect.poll(async () => (await state(first)).id, { timeout: 10_000 }).not.toBeNull();
  const draft = (await state(first)).id!;

  const second = await context.newPage();
  second.on("pageerror", (e) => errors.push(e.message));
  await second.goto("/");
  const blocked = second.getByTestId("other-tab");
  await expect(blocked).toBeVisible({ timeout: 30_000 });
  await expect(blocked.getByRole("heading")).toHaveText("ethereal is open in another tab");
  expect(await engineStarted(second)).toBe(false);

  // The first tab's draft is still there: an edit makes it a project, and it saves.
  await first.keyboard.press("Escape");
  await expect(projectScreen(first)).toHaveCount(0);
  await createTrack(first, "Midi");
  await expect.poll(async () => (await state(first)).dirty, { timeout: 10_000 }).toBe(false);
  expect((await state(first)).id).toBe(draft);
  await expect(first.locator(".eth-project__error")).toHaveCount(0);

  // An edit right before "use here" is saved by the handover (the autosave wait is 1 s).
  await createTrack(first, "Audio");
  const tracks = (await state(first)).tracks;
  await blocked.getByRole("button", { name: "use here" }).click();
  await expect(first.getByTestId("other-tab").getByRole("heading")).toHaveText("ethereal moved to another tab", { timeout: 30_000 });
  await expect(blocked).toHaveCount(0, { timeout: 30_000 });
  expect(await engineStarted(second)).toBe(true);
  // The second tab has the first one's project, with every track.
  await expect.poll(async () => (await state(second)).projects, { timeout: 30_000 }).toContain(draft);
  const picker = projectScreen(second);
  await expect(picker).toBeVisible({ timeout: 30_000 });
  await picker
    .getByRole("list", { name: "stored projects" })
    .getByRole("button", { name: /^open / })
    .first()
    .click();
  await expect.poll(async () => (await state(second)).id, { timeout: 30_000 }).toBe(draft);
  expect((await state(second)).tracks).toBe(tracks);

  // And back: the first tab takes it again (a fresh load), the second one steps aside.
  await first.getByRole("button", { name: "use here" }).click();
  await expect(second.getByTestId("other-tab")).toBeVisible({ timeout: 30_000 });
  await expect(first.getByTestId("other-tab")).toHaveCount(0, { timeout: 30_000 });
  // It reloads at the project's address, so the project opens again.
  await expect.poll(async () => (await state(first)).id, { timeout: 30_000 }).toBe(draft);
  expect((await state(first)).tracks).toBe(tracks);
  expect(errors).toEqual([]);
});
