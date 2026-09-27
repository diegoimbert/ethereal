// web-perf: a larger project on the web build, against the real engine. Every graph
// snapshot reaches the AudioWorklet as a binary `Publish` frame (ether_core::codec).
//
// One MIDI track (synth, 8 clips of 16 notes) duplicated to 24 tracks (192 clips) through
// the real controller, then played while 20 more edits republish the graph: every
// duplicated track sounds (its snapshot decoded and compiled in the worklet), the audio
// clock keeps moving, and the engine reports no errors (a bad graph frame would surface as an
// "audio engine: …" warning).
//
// Commands are posted straight to the controller Worker (`window.__etherEngine`) with ids
// the UI transport never uses (it ignores replies it didn't ask for); the UI mirror
// (`window.__ether`) still receives every patch. No sleeps: each step waits on replies or
// engine state.
import { expect, test, type Page } from "@playwright/test";
import type { Command, Project } from "@/generated";
import { playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

interface Probe {
  replies: Record<number, { status: string; error?: unknown }>;
  warnings: string[];
  post(json: string): void;
}

const TRACKS = 24;
const CLIPS = 8;
const NOTES = 16;

/** Deterministic ULIDs: prefix + decimal digits (valid Crockford base32). */
const ulid = (kind: number, n: number) => `01J${kind}${String(n).padStart(22, "0")}`;

async function installProbe(page: Page) {
  await page.evaluate(() => {
    const ep = (
      window as unknown as {
        __etherEngine: {
          handles(): { controller: Worker } | null;
          post(json: string): void;
        };
      }
    ).__etherEngine;
    const probe: Probe = { replies: {}, warnings: [], post: (json) => ep.post(json) };
    ep.handles()!.controller.addEventListener("message", (e: MessageEvent<{ type: string; json?: string }>) => {
      if (e.data.type !== "server" || !e.data.json) return;
      for (const m of JSON.parse(e.data.json) as { kind: string; body: Record<string, unknown> }[]) {
        if (m.kind === "Reply") {
          const { id, result } = m.body as { id: number; result: { status: string; error?: unknown } };
          probe.replies[id] = result;
        } else if (m.kind === "Event") {
          const ev = m.body as { type: string; message?: string };
          if (ev.type === "Notification" && ev.message?.startsWith("audio engine:")) probe.warnings.push(ev.message);
        }
      }
    });
    (window as unknown as { __webPerf: Probe }).__webPerf = probe;
  });
}

let nextId = 1_000_000_000;

/** Send a command and wait for its (successful) reply. */
async function send(page: Page, command: Command) {
  const id = nextId++;
  await page.evaluate(
    ([id, command]) => {
      (window as unknown as { __webPerf: Probe }).__webPerf.post(JSON.stringify({ id, gesture: null, command }));
    },
    [id, command] as const,
  );
  await expect
    .poll(() => page.evaluate((id) => (window as unknown as { __webPerf: Probe }).__webPerf.replies[id], id), {
      timeout: 15_000,
    })
    .toMatchObject({ status: "Ok" });
}

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

/** The AudioContext clock (advances while the worklet renders). */
const audioSeconds = (page: Page): Promise<number> =>
  page.evaluate(() => {
    const ep = (window as unknown as { __etherEngine: { handles(): { context: AudioContext } | null } })
      .__etherEngine;
    return ep.handles()?.context.currentTime ?? 0;
  });

test("large project: binary graph snapshots play while editing", async ({ page }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project !== null), {
      timeout: 30_000,
    })
    .toBe(true);
  await installProbe(page);

  // Track 0: synth + 8 clips of 16 notes, looped over 32 beats.
  const first = ulid(1, 0);
  await send(page, {
    domain: "Track",
    command: { type: "Create", id: first, kind: "Midi", name: "perf 0", color: null, parent: null, before: null },
  });
  await send(page, {
    domain: "Device",
    command: {
      type: "Insert",
      id: ulid(2, 0),
      track: first,
      device: { type: "Builtin", device: { type: "Synth" } },
      before: null,
    },
  });
  for (let c = 0; c < CLIPS; c++) {
    const clip = ulid(3, c);
    await send(page, {
      domain: "Clip",
      command: { type: "CreateMidi", id: clip, track: first, start: c * 4, length: 4, name: null },
    });
    await send(page, {
      domain: "Note",
      command: {
        type: "Add",
        clip,
        notes: Array.from({ length: NOTES }, (_, k) => ({
          id: ulid(4, c * NOTES + k),
          pitch: 48 + ((k * 5) % 24),
          velocity: 0.8,
          start: k * 0.25,
          duration: 0.2,
        })),
      },
    });
  }
  const tracks = [first];
  for (let t = 1; t < TRACKS; t++) {
    const id = ulid(1, t);
    await send(page, { domain: "Track", command: { type: "Duplicate", id: first, new_id: id } });
    tracks.push(id);
  }
  const doc = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);
  expect(Object.values(doc.clips).filter((c) => tracks.includes(c.track)).length).toBe(TRACKS * CLIPS);

  await send(page, { domain: "Transport", command: { type: "SetLoopRegion", region: { start: 0, end: 32 } } });
  await send(page, { domain: "Transport", command: { type: "SetLoopEnabled", enabled: true } });
  await playButton(page).click();
  await expect(page.getByRole("button", { name: "Stop" }).first()).toBeVisible();

  // Every track sounds: its part of the snapshot was decoded and compiled in the worklet.
  for (const t of [tracks[0]!, tracks[TRACKS / 2]!, tracks[TRACKS - 1]!]) {
    await expect.poll(() => peakOf(page, t), { timeout: 15_000 }).toBeGreaterThan(0.01);
  }

  // Republish the (large) graph 20 times while playing: move a clip back and forth.
  const moved = ulid(3, 0);
  const t0 = await audioSeconds(page);
  for (let i = 0; i < 20; i++) {
    await send(page, {
      domain: "Clip",
      command: { type: "Move", moves: [{ id: moved, track: first, start: i % 2 === 0 ? 0.5 : 0 }] },
    });
  }
  await expect.poll(() => audioSeconds(page), { timeout: 10_000 }).toBeGreaterThan(t0 + 0.2);
  await expect.poll(() => peakOf(page, tracks[TRACKS - 1]!), { timeout: 10_000 }).toBeGreaterThan(0.01);

  const warnings = await page.evaluate(() => (window as unknown as { __webPerf: Probe }).__webPerf.warnings);
  expect(warnings).toEqual([]);
  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
