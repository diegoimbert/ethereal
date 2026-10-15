import { expect, test } from "@playwright/test";
import { launch } from "./ui";

// The other specs start from an empty project (playwright.config.ts: `eth.project.empty`);
// this one doesn't, so "New project" makes the starter project.
test.use({ storageState: { cookies: [], origins: [] } });

test("a new project starts with a MIDI track “synth” playing the built-in Synth", async ({ page }) => {
  await launch(page, "Starter");
  const header = page.getByLabel("synth track", { exact: true });
  await expect(header).toBeVisible();
  const project = await page.evaluate(() => (window as unknown as { __ether: { state(): { project: unknown } } }).__ether.state().project);
  const p = project as { tracks: Record<string, { id: string; kind: string; name: string }>; devices: Record<string, { track: string; kind: unknown }> };
  const tracks = Object.values(p.tracks);
  expect(tracks.map((t) => t.kind).sort()).toEqual(["Master", "Midi"]);
  const synth = tracks.find((t) => t.kind === "Midi")!;
  expect(Object.values(p.devices).map((d) => [d.track, d.kind])).toEqual([[synth.id, { type: "Builtin", device: { type: "Synth" } }]]);
});
