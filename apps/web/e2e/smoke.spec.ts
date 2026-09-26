// Smoke test: the web build boots the wasm engine (controller Worker + AudioWorklet over
// SharedArrayBuffer rings, OPFS store), and playing advances the playhead rendered by
// ether-core, with the real EtherController.
import { expect, test, type Locator, type Page } from "@playwright/test";

/** Seconds shown by the transport bar (`m:ss.mmm`). */
async function shownSeconds(position: Locator): Promise<number> {
  const text = (await position.textContent()) ?? "";
  const m = /^(\d+):(\d+)\.(\d+)$/.exec(text.trim());
  if (!m) throw new Error(`unexpected position text ${JSON.stringify(text)}`);
  return Number(m[1]) * 60 + Number(m[2]) + Number(m[3]) / 1000;
}

async function storedProjects(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const projects = await (await root.getDirectoryHandle("ethereal")).getDirectoryHandle("projects");
    const names: string[] = [];
    for await (const [name] of projects.entries()) names.push(name);
    return names;
  });
}

test("plays with the wasm engine and the playhead advances", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  expect(await page.evaluate(() => window.crossOriginIsolated)).toBe(true);

  const play = page.getByRole("button", { name: "Play" });
  await expect(play).toBeVisible({ timeout: 30_000 });
  const position = page.getByTestId("position-time");

  await play.click();
  await expect(page.getByRole("button", { name: "Stop" }).first()).toBeVisible();

  // The AudioContext runs (the worklet renders).
  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const ep = (window as unknown as { __etherEngine?: { handles(): { context: AudioContext } | null } })
            .__etherEngine;
          return ep?.handles()?.context.state;
        }),
      { timeout: 10_000 },
    )
    .toBe("running");

  // The UI playhead (Playhead frames from the engine) keeps rising.
  await expect.poll(() => shownSeconds(position), { timeout: 10_000 }).toBeGreaterThan(0);
  const t1 = await shownSeconds(position);
  await expect.poll(() => shownSeconds(position), { timeout: 10_000 }).toBeGreaterThan(t1 + 0.2);

  // The project was persisted to OPFS by the engine-side store.
  await expect.poll(() => storedProjects(page).then((p) => p.length), { timeout: 10_000 }).toBeGreaterThan(0);

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

