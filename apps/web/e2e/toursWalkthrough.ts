// The guided tours walked through like a user (the tests: walkthroughTours) (docs/TUTORIALS.md "Tests"), every tour, every
// step, at a laptop size and two smaller ones. Opt-in (about a minute with 2 workers):
// `just e2e-tours`, the `tours` Playwright project (tours.walkthrough.spec.ts), only defined with ETHEREAL_TOURS=1. Run it
// before a release and after UI refactors.
//
// For each step it checks what breaks tours in practice:
// - the highlighted parts are found (no "we'll skip it" fallback), inside the window, and a
//   press at their center reaches them (nothing covers them: a menu, a dialog, the bubble);
// - the ring is the topmost element along its edge (no layer hides the highlight, #753);
// - the bubble is inside the window and nothing in it is clipped or needs scrolling;
// - the step moves on after the action it asks for, done the way a user does it (a click on
//   "share", "add track" then "create audio track", the arm button), within STEP_TIMEOUT;
// and at the end, the tour is done, the welcome screen is back and no console error showed.
//
// The browser build runs the web branch of each tour: the desktop-only steps (audio devices,
// recording a take: `when` guards in tours/recordAudio.tsx) need the desktop app and are
// covered by the static checks (ui/src/features/tutorials/tours.static.test.tsx). The mobile
// shell offers no tour: checked below.
import { expect as baseExpect, test, type Page, type TestInfo } from "@playwright/test";
import type { Project } from "@/generated";
import { openClip } from "./clips";
import { IPHONE, mobileShell } from "./mobile";
import { projectScreen } from "./ui";

const expect = baseExpect.configure({ timeout: 15_000 });

/** How long a step may take to move on after its action. */
const STEP_TIMEOUT = 10_000;
/** The engine's grace before a missing target shows the fallback (store.ts MISSING_GRACE_MS), plus slack. */
const FOUND_TIMEOUT = 5_000;

export type Action = (page: Page) => Promise<void>;
/** What the user does on a step. A step left out gets "next" (or "finish" on the last one); a waiting step must be listed. */
export type Script = Readonly<Record<string, Action>>;
/** A tour of the welcome screen and how a user walks it. */
export type Tour = { title: string; id: string; script: Script; example?: true };

const bubble = (page: Page) => page.getByTestId("tour-bubble");
const tourEl = (page: Page) => page.getByTestId("tour");
const press = (name: string) => (page: Page) => bubble(page).getByRole("button", { name, exact: true }).click();

/** The play button: the example tours wait for playback. */
export const play: Action = (page) => page.getByRole("toolbar", { name: "transport" }).getByRole("button", { name: "play", exact: true }).click();


/** The open project's first MIDI track and its first clip (the first-track tour makes them). */
async function firstMidi(page: Page): Promise<{ track: string; clip: string | null }> {
  const p = await page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project);
  const track = Object.values(p?.tracks ?? {}).find((t) => t.kind === "Midi")!;
  const clip = Object.values(p?.clips ?? {}).find((c) => c.track === track.id);
  return { track: track.id, clip: clip?.id ?? null };
}

/** The first-track tour (#14): a MIDI track, an instrument, a clip, three notes, play and stop. */
const FIRST_TRACK: Script = {
  "add-track": async (page) => {
    await page.locator('[data-tour="add-track"]').click();
    await page.getByRole("menuitem", { name: "create midi track" }).click();
  },
  instrument: (page) => page.locator('[data-tour="instrument-pick"]').getByRole("button", { name: "poly synth", exact: true }).click(),
  clip: async (page) => {
    const { track } = await firstMidi(page);
    await page.locator(`[data-lane="${track}"]`).dblclick({ position: { x: 10, y: 20 } });
  },
  "open-clip": async (page) => openClip(page, (await firstMidi(page)).clip!),
  notes: async (page) => {
    // Inside the clip (left of the "outside" shade), like flow.spec.ts.
    const grid = page.getByTestId("piano-roll-grid");
    const box = (await grid.boundingBox())!;
    const end = (await page.getByTestId("piano-roll-clip-end").boundingBox())!;
    const width = end.x - box.x;
    for (const [fx, fy] of [
      [0.05, 0.4],
      [0.35, 0.45],
      [0.65, 0.5],
    ] as const)
      await grid.dblclick({ position: { x: width * fx + 2, y: box.height * fy } });
  },
  play: (page) => page.locator('[data-tour="play"]').click(),
  stop: (page) => page.locator('[data-tour="play"]').click(),
};

/**
 * The web walk of every tour on this build's welcome screen, by its item's title. `example`: an example
 * beat's tour (#518), started by its "show me how" button in the examples section.
 */
export const TOURS: ReadonlyArray<Tour> = [
  { title: "make your first track", id: "first-track", script: FIRST_TRACK },
  {
    title: "record audio",
    id: "record-audio",
    script: {
      "open-settings": (page) => page.getByRole("button", { name: "settings", exact: true }).click(),
      // Skipped when the settings open on the audio tab (the default).
      "audio-tab": (page) => page.getByRole("dialog", { name: "settings", exact: true }).locator('[data-tour="settings-tab-audio"]').click(),
      "close-settings": (page) => page.getByRole("dialog", { name: "settings", exact: true }).getByRole("button", { name: "done" }).click(),
      "add-track": async (page) => {
        await page.locator('[data-tour="add-track"]').click();
        await page.getByRole("menuitem", { name: "create audio track" }).click();
      },
      inputs: (page) => page.locator('[data-tour="rec-inputs"]').click(),
      arm: (page) => page.locator('[data-tour="rec-inputs-panel"]').getByRole("button", { name: /^arm / }).first().click(),
    },
  },
  { title: "house beat", id: "example-house-beat", example: true, script: { play } },
  { title: "hip-hop loop", id: "example-hip-hop-loop", example: true, script: { play } },
  { title: "lo-fi chords", id: "example-lofi-chords", example: true, script: { play } },
  { title: "simple bassline", id: "example-simple-bassline", example: true, script: { play } },
];

/** The welcome screen's button that starts a tour (accessible name), and the same once the tour is done. */
const startName = (tour: Tour) => (tour.example ? `show me how: ${tour.title}` : tour.title);
const doneName = (tour: Tour) => `${startName(tour)} (completed)`;

/** A common laptop, a small one, and a small window. */
const SIZES = [
  { width: 1280, height: 800 },
  { width: 1024, height: 640 },
  { width: 800, height: 600 },
] as const;

/** Wait for the ring(s) and the bubble to stop moving (they animate from step to step). */
async function settle(page: Page): Promise<void> {
  const boxes = () =>
    page.evaluate(() =>
      [...document.querySelectorAll('[data-testid="tour-ring"], [data-testid="tour-bubble"]')]
        .map((el) => {
          const r = el.getBoundingClientRect();
          return `${Math.round(r.x)},${Math.round(r.y)},${Math.round(r.width)},${Math.round(r.height)}`;
        })
        .join(" "),
    );
  let last = "";
  await expect
    .poll(
      async () => {
        const now = await boxes();
        const same = now === last;
        last = now;
        return same;
      },
      { intervals: [150], timeout: STEP_TIMEOUT },
    )
    .toBe(true);
}

/** The layout problems of the current step, as messages (empty: fine). Runs in the page. */
function layoutProblems(): string[] {
  const problems: string[] = [];
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const tour = document.querySelector<HTMLElement>('[data-testid="tour"]')!;
  const bubble = document.querySelector<HTMLElement>('[data-testid="tour-bubble"]')!;
  const inside = (r: DOMRect) => r.left >= -0.5 && r.top >= -0.5 && r.right <= vw + 0.5 && r.bottom <= vh + 0.5;
  const fmt = (r: DOMRect) => `${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.width)}x${Math.round(r.height)}`;
  const describe = (el: Element | null) =>
    el === null
      ? "nothing"
      : `<${el.tagName.toLowerCase()}${el.id ? `#${el.id}` : ""}${el.getAttribute("data-testid") ? ` data-testid=${el.getAttribute("data-testid")}` : ""} class="${el.getAttribute("class") ?? ""}">`;

  // The bubble: inside the window, nothing clipped, no scrolling needed.
  const b = bubble.getBoundingClientRect();
  if (!inside(b)) problems.push(`the bubble (${fmt(b)}) leaves the ${vw}x${vh} window`);
  if (bubble.scrollHeight > bubble.clientHeight + 1) problems.push(`the bubble's text needs scrolling (${bubble.scrollHeight}px in ${bubble.clientHeight}px)`);
  for (const el of bubble.querySelectorAll<HTMLElement>("*")) {
    const style = getComputedStyle(el);
    // Ellipsis on the tour's title is by design.
    if (el.classList.contains("eth-tour__tour")) continue;
    if (el.scrollWidth > el.clientWidth + 1 && style.overflowX !== "visible" && el.clientWidth > 0)
      problems.push(`clipped text in the bubble: ${describe(el)} (${el.scrollWidth}px in ${el.clientWidth}px)`);
  }
  const bubbleRect = b;

  // The named targets: inside the window, and a press at their center reaches them.
  const names = (tour.dataset.targets ?? "").split(" ").filter(Boolean);
  for (const name of names) {
    for (const el of document.querySelectorAll(`[data-tour="${name}"]`)) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) continue;
      if (!inside(r)) problems.push(`target "${name}" (${fmt(r)}) is not fully inside the ${vw}x${vh} window`);
      const cx = Math.min(Math.max(r.left + r.width / 2, 0), vw - 1);
      const cy = Math.min(Math.max(r.top + r.height / 2, 0), vh - 1);
      const top = document.elementFromPoint(cx, cy);
      if (!top || !(el === top || el.contains(top))) problems.push(`target "${name}" is covered at its center by ${describe(top)}`);
      const overlap = !(r.right <= bubbleRect.left || r.left >= bubbleRect.right || r.bottom <= bubbleRect.top || r.top >= bubbleRect.bottom);
      if (overlap) problems.push(`the bubble covers target "${name}"`);
    }
  }

  // The rings: the topmost element along their edges (hit-testing turned on just for this).
  const rings = [...document.querySelectorAll<HTMLElement>('[data-testid="tour-ring"]')];
  const layer = rings[0]?.parentElement;
  if (layer) {
    const before = layer.style.pointerEvents;
    layer.style.pointerEvents = "auto";
    for (const ring of rings) ring.style.pointerEvents = "auto";
    for (const [i, ring] of rings.entries()) {
      const r = ring.getBoundingClientRect();
      if (r.width < 4 || r.height < 4) problems.push(`ring ${i} is degenerate (${fmt(r)})`);
      const edges: Array<[number, number]> = [
        [r.left + 1, r.top + r.height / 2],
        [r.right - 1, r.top + r.height / 2],
        [r.left + r.width / 2, r.top + 1],
        [r.left + r.width / 2, r.bottom - 1],
      ];
      for (const [x, y] of edges) {
        if (x < 0 || y < 0 || x >= vw || y >= vh) continue;
        const top = document.elementFromPoint(x, y);
        if (top !== ring) problems.push(`ring ${i} is hidden at (${Math.round(x)}, ${Math.round(y)}) by ${describe(top)}`);
      }
    }
    for (const ring of rings) ring.style.pointerEvents = "";
    layer.style.pointerEvents = before;
  }
  return problems;
}

async function failWith(page: Page, info: TestInfo, message: string): Promise<never> {
  await info.attach("screenshot", { body: await page.screenshot(), contentType: "image/png" });
  throw new Error(message);
}

async function walk(page: Page, info: TestInfo, tour: Tour): Promise<string[]> {
  const walked: string[] = [];
  const screen = projectScreen(page);
  await expect(screen).toBeVisible({ timeout: 30_000 });
  await screen.getByRole("button", { name: startName(tour), exact: true }).click();
  await expect(tourEl(page)).toHaveAttribute("data-tour-id", tour.id, { timeout: 30_000 });

  for (let guard = 0; guard < 60; guard++) {
    if ((await tourEl(page).count()) === 0) return walked;
    const id = (await tourEl(page).getAttribute("data-step"))!;
    const at = `tour "${tour.id}", step "${id}"`;
    walked.push(id);

    // Its highlighted parts are found (else the engine shows the fallback after a grace).
    const targets = await tourEl(page).getAttribute("data-targets");
    if (targets !== null) {
      try {
        await expect(page.getByTestId("tour-ring").first()).toBeVisible({ timeout: FOUND_TIMEOUT });
      } catch {
        await failWith(page, info, `${at}: no highlight (targets "${targets || "found by code"}" not on screen)`);
      }
      // Stays found past the grace (a target flickering in and out).
      await page.waitForTimeout(1_300);
      if ((await tourEl(page).getAttribute("data-missing")) !== null) await failWith(page, info, `${at}: its targets went missing (the fallback shows)`);
    }
    await settle(page);
    const problems = await page.evaluate(layoutProblems);
    if (problems.length > 0) await failWith(page, info, `${at} at ${page.viewportSize()!.width}x${page.viewportSize()!.height}:\n- ${problems.join("\n- ")}`);

    // The user's action.
    const waiting = (await tourEl(page).getAttribute("data-waiting")) !== null;
    const scripted = tour.script[id];
    if (scripted) await scripted(page);
    else if (waiting) await failWith(page, info, `${at} waits for the app, and the walkthrough has no action for it: add one to its script`);
    else {
      const finish = bubble(page).getByRole("button", { name: "finish", exact: true });
      await ((await finish.count()) > 0 ? finish.click() : press("next")(page));
    }

    // It moves on.
    try {
      await expect
        .poll(async () => ((await tourEl(page).count()) === 0 ? null : await tourEl(page).getAttribute("data-step")), { timeout: STEP_TIMEOUT })
        .not.toBe(id);
    } catch {
      await failWith(page, info, `${at}: didn't move on within ${STEP_TIMEOUT / 1000} s after its action`);
    }
  }
  return failWith(page, info, `tour "${tour.id}" didn't end after 60 steps (${walked.join(", ")})`);
}

/** The walkthrough tests of `tours` (every tour of the welcome screen). */
export function walkthroughTours(tours: ReadonlyArray<Tour>): void {
  // Every test starts its own page and tour: they run side by side (`just e2e-tours` uses 2 workers).
  test.describe.configure({ mode: "parallel" });

  for (const size of SIZES)
    test.describe(`tours at ${size.width}x${size.height}`, () => {
      test.use({ viewport: size });

      for (const tour of tours)
        test(`${tour.id}: every step, start to finish`, async ({ page }, info) => {
          const errors: string[] = [];
          page.on("pageerror", (e) => errors.push(`page error: ${e.message}`));
          page.on("console", (m) => {
            if (m.type() === "error") errors.push(`console error: ${m.text()}`);
          });
          await page.goto("/");
          const walked = await walk(page, info, tour);
          info.annotations.push({ type: "steps", description: walked.join(" → ") });

          // Done: the welcome screen is back with the item checked.
          await expect(tourEl(page)).toHaveCount(0);
          const screen = projectScreen(page);
          await expect(screen).toBeVisible();
          await expect(screen.getByRole("button", { name: doneName(tour), exact: true })).toBeVisible();
          expect(errors).toEqual([]);
        });
    });

  test("every tour on the welcome screen has a walkthrough", async ({ page }) => {
    await page.goto("/");
    const screen = projectScreen(page);
    await expect(screen).toBeVisible({ timeout: 30_000 });
    const section = screen.getByRole("region", { name: "get started" });
    await expect(section.locator(".eth-welcome__article--tour").first()).toBeEnabled({ timeout: 30_000 });
    const offered = (
      await section.locator(".eth-welcome__article--tour:not([data-testid])").evaluateAll((els) => els.map((el) => el.getAttribute("aria-label")))
    ).sort();
    expect(offered).toEqual(
      tours
        .filter((t) => !t.example)
        .map((t) => t.title)
        .sort(),
    );
    // The example beats' tours (#518): each example's "show me how".
    const examples = screen.getByRole("region", { name: "examples" });
    const guides = examples.getByRole("button", { name: /^show me how: / });
    await expect(guides.first()).toBeEnabled({ timeout: 30_000 });
    const guided = (await guides.evaluateAll((els) => els.map((el) => el.getAttribute("aria-label")))).sort();
    expect(guided).toEqual(
      tours
        .filter((t) => t.example)
        .map(startName)
        .sort(),
    );
  });

  test.describe("on the mobile shell", () => {
    test.use({ ...IPHONE });

    test("offers no tour (it has no tour overlay; tours declare desktop and web)", async ({ page }) => {
      test.info().annotations.push({ type: "skip-reason", description: "no tour runs on mobile: the items must say soon instead of starting a broken tour" });
      await page.goto("/?mobile=1");
      await expect(mobileShell(page)).toBeVisible({ timeout: 30_000 });
      const screen = projectScreen(page);
      await expect(screen).toBeVisible({ timeout: 30_000 });
      const section = screen.getByRole("region", { name: "get started" });
      for (const t of tours.filter((t) => !t.example)) {
        const item = section.locator(".eth-welcome__article", { hasText: t.title });
        await expect(item).toHaveAttribute("aria-disabled", "true");
        await expect(item).toContainText("soon");
      }
      // The examples still open there, without their tours.
      const examples = screen.getByRole("region", { name: "examples" });
      await expect(examples.getByRole("button", { name: "open house beat", exact: true })).toBeVisible();
      await expect(examples.getByRole("button", { name: /^show me how/ })).toHaveCount(0);
    });
  });
}
