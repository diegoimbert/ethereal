// Recording on the web build (real wasm controller + engine): the browser host has no audio
// or MIDI inputs, so `Recording::ListInputs` is `Unsupported` and the record button is
// disabled with an explanatory tooltip. The document-side recording settings still work:
// count-in (undoable), per-track input and monitoring, and record-arm (runtime state).
import { expect, test, type Page } from "@playwright/test";
import type { Project, TrackId } from "@/generated";

interface Handle {
  state(): { project: Project | null; armedTracks: TrackId[] };
}

const state = (page: Page) => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state());

async function doc(page: Page): Promise<Project> {
  const p = (await state(page)).project;
  if (!p) throw new Error("no project open");
  return p;
}

test("recording controls on the web: inputs unsupported, settings and arm work", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => state(page).then((s) => s.project !== null), { timeout: 30_000 }).toBe(true);

  // No inputs in the browser: record is disabled, and says why.
  const record = page.getByRole("button", { name: "Record armed tracks" });
  await expect(record).toBeDisabled();
  await expect(record).toHaveAttribute("title", /desktop app/);
  await expect(page.getByRole("button", { name: "Punch in/out" })).toBeDisabled();

  // Count-in is an undoable project setting handled by the wasm controller.
  await page.getByRole("combobox", { name: "Count-in" }).click();
  await page.getByRole("option", { name: "2 bars" }).click();
  await expect.poll(async () => (await doc(page)).settings.count_in_bars).toBe(2);

  // An audio track, armed and set to monitor "In" from the inputs panel.
  await page.getByRole("button", { name: /New track/ }).click();
  await page.getByRole("button", { name: "Create audio track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const audio = Object.values(await doc(page).then((p) => p.tracks)).find((t) => t.kind === "Audio")!;
  await page.getByRole("button", { name: "Inputs" }).click();
  const panel = page.getByRole("dialog", { name: "Recording inputs" });
  await expect(panel).toContainText("desktop app");
  const row = panel.locator(`tr[data-track="${audio.id}"]`);
  await row.getByRole("button", { name: `Arm ${audio.name}` }).click();
  await expect.poll(() => state(page).then((s) => s.armedTracks)).toContain(audio.id);
  await row.getByRole("combobox", { name: `Monitoring of ${audio.name}` }).click();
  await page.getByRole("option", { name: "In", exact: true }).click();
  await expect.poll(async () => (await doc(page)).tracks[audio.id]!.monitor).toBe("In");

  expect(errors).toEqual([]);
});
