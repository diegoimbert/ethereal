// The example beats (#518, docs/TUTORIALS.md "Example beats"): each example on the welcome
// screen opens as a new project from its factory template (`factory/example-…`) and plays:
// every track is heard on its meter (the drum rack's pads, the instruments and their
// effects, in the real wasm engine). Help (the palette's "example beats", the project menu)
// brings the welcome screen and its examples back. The tours are walked in
// tours.walkthrough.spec.ts (`just e2e-tours`).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { paletteCommand, playButton, projectMenu, projectScreen } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

/** The examples, by their title on the welcome screen, with the tracks each one has. */
const EXAMPLES = [
  { title: "house beat", tracks: ["drums", "bass"], bpm: 124 },
  { title: "hip-hop loop", tracks: ["drums", "keys"], bpm: 90 },
  { title: "lo-fi chords", tracks: ["chords", "drums"], bpm: 80 },
  { title: "simple bassline", tracks: ["drums", "bass"], bpm: 120 },
] as const;

for (const ex of EXAMPLES)
  test(`${ex.title}: opens from the welcome screen and every track plays`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto("/");
    const screen = projectScreen(page);
    await expect(screen).toBeVisible({ timeout: 30_000 });
    const examples = screen.getByRole("region", { name: "examples" });
    await examples.getByRole("button", { name: `open ${ex.title}`, exact: true }).click();
    await expect(screen).toBeHidden({ timeout: 30_000 });

    // The example's project: named after it, its tracks, a looped tempo.
    await expect.poll(async () => (await project(page))?.settings.name, { timeout: 30_000 }).toBe(ex.title);
    const p = (await project(page))!;
    expect(p.settings.loop_enabled).toBe(true);
    const tracks = Object.values(p.tracks)
      .filter((t) => t.kind === "Midi")
      .sort((a, b) => (a.order < b.order ? -1 : 1));
    expect(tracks.map((t) => t.name)).toEqual(ex.tracks);
    expect(Object.values(p.tempo_points).map((pt) => pt.bpm)).toEqual([ex.bpm]);

    // Every track is heard while it plays (the peaks hold for a moment: poll each one).
    await playButton(page).click();
    for (const t of tracks) await expect.poll(() => peakOf(page, t.id), { timeout: 15_000, message: `${ex.title}: ${t.name} is silent` }).toBeGreaterThan(0.02);
    await page.getByTestId("transport-stop").click();
    expect(errors).toEqual([]);
  });

test("help: example beats brings back the welcome screen's examples", async ({ page }) => {
  await page.goto("/");
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("region", { name: "examples" }).getByRole("button", { name: "open simple bassline", exact: true }).click();
  await expect(screen).toBeHidden({ timeout: 30_000 });
  await paletteCommand(page, "example beats");
  await expect(screen.getByRole("region", { name: "examples" })).toBeVisible();
  await screen.getByRole("button", { name: "close" }).first().click();
  await expect(screen).toBeHidden();
  await projectMenu(page, "example beats");
  await expect(screen.getByRole("region", { name: "examples" })).toBeVisible();
});
