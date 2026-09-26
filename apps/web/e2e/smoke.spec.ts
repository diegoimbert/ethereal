// Smoke test: the web build boots the wasm engine (controller Worker + AudioWorklet over
// SharedArrayBuffer rings, OPFS store), and playing advances the playhead rendered by
// ether-core. Uses the thin fake controller (`?controller=fake`) until EtherController is
// implemented; the engine, rings and store are the real ones.
import { expect, test } from "@playwright/test";

test("plays with the wasm engine and the playhead advances", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/?controller=fake");
  expect(await page.evaluate(() => window.crossOriginIsolated)).toBe(true);

  const play = page.getByRole("button", { name: "Play" });
  await expect(play).toBeVisible({ timeout: 30_000 });
  const position = page.getByTestId("position-time");
  const before = await position.textContent();

  await play.click();
  await expect(page.getByRole("button", { name: "Stop" }).first()).toBeVisible();

  // The AudioWorklet is rendering (engine-side block counter reported over the rings).
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

  // The UI playhead (fed by Playhead frames from the engine) moves.
  await expect.poll(() => position.textContent(), { timeout: 10_000 }).not.toBe(before);
  const t1 = await position.textContent();
  await page.waitForTimeout(500);
  const t2 = await position.textContent();
  expect(t2).not.toBe(t1);

  // The project was persisted to OPFS by the engine-side store.
  const stored = await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const projects = await (await root.getDirectoryHandle("ethereal")).getDirectoryHandle("projects");
    const names: string[] = [];
    for await (const [name] of projects.entries()) names.push(name);
    return names;
  });
  expect(stored.length).toBeGreaterThan(0);

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError/.test(e))).toEqual([]);
});
