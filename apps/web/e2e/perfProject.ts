// Shared setup of the frame-rate specs (`*.perf.spec.ts`): a project of 64 MIDI tracks with
// 8 clips of 16 notes each, built through the engine directly (fast, no UI gestures), plus
// frame-interval and layer-count probes. The UI mirror is `window.__ether`.
import { availableParallelism, loadavg } from "node:os";
import { expect, type CDPSession, type Page } from "@playwright/test";
import type { Command, Project } from "@/generated";
import { expandFolder, openLibrary } from "./ui";

/** Drops the library's bass loop on `track`'s lane; resolves with its media id. */
export async function importBassLoop(page: Page, track: string): Promise<string> {
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const sample = page.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${track}"]`), { targetPosition: { x: 5, y: 20 } });
  let media: string | undefined;
  await expect
    .poll(
      async () => {
        const c = Object.values((await doc(page)).clips).find((c) => c.content.type === "Audio");
        media = c?.content.type === "Audio" ? c.content.media : undefined;
        return !!media;
      },
      { timeout: 30_000 },
    )
    .toBe(true);
  return media!;
}

/** Sets the arranger's zoom (px per beat) and horizontal scroll (beats). */
export async function setViewport(page: Page, pxPerBeat: number, scrollBeats: number): Promise<void> {
  await page.evaluate(
    ([ppb, scroll]) =>
      (window as unknown as { __ether: { arrangementView(): { getState(): { setViewport(v: object): void } } } }).__ether
        .arrangementView()
        .getState()
        .setViewport({ pxPerBeat: ppb, scrollBeats: scroll }),
    [pxPerBeat, scrollBeats] as const,
  );
}

interface Handle {
  state(): { project: Project | null };
}

export const doc = async (page: Page): Promise<Project> => {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
};

interface Probe {
  replies: Record<number, { status: string }>;
  post(json: string): void;
}
/** A deterministic id (`kind` keeps tracks, clips and notes apart). */
export const ulid = (kind: number, n: number): string => `01J${kind}${String(n).padStart(22, "0")}`;
let nextId = 2_000_000_000;

export async function send(page: Page, command: Command): Promise<void> {
  const id = nextId++;
  await page.evaluate(([id, command]) => (window as unknown as { __probe: Probe }).__probe.post(JSON.stringify({ id, gesture: null, command })), [
    id,
    command,
  ] as const);
  await expect
    .poll(() => page.evaluate((id) => (window as unknown as { __probe: Probe }).__probe.replies[id], id), { timeout: 15_000 })
    .toMatchObject({ status: "Ok" });
}

/** Lets `send` post commands to the engine and wait for their replies. */
export async function installProbe(page: Page): Promise<void> {
  await page.evaluate(() => {
    const ep = (window as unknown as { __etherEngine: { handles(): { controller: Worker } | null; post(json: string): void } }).__etherEngine;
    const probe: Probe = { replies: {}, post: (json) => ep.post(json) };
    ep.handles()!.controller.addEventListener("message", (e: MessageEvent<{ type: string; json?: string }>) => {
      if (e.data.type !== "server" || !e.data.json) return;
      for (const m of JSON.parse(e.data.json) as { kind: string; body: { id: number; result: { status: string } } }[]) {
        if (m.kind === "Reply") probe.replies[m.body.id] = m.body.result;
      }
    });
    (window as unknown as { __probe: Probe }).__probe = probe;
  });
}

/** In the open project: one MIDI track ("perf 0") with 8 clips of 16 notes, duplicated to `tracks`. */
export async function buildPerfProject(page: Page, tracks = 64): Promise<void> {
  await installProbe(page);
  const first = ulid(1, 0);
  await send(page, { domain: "Track", command: { type: "Create", id: first, kind: "Midi", name: "perf 0", color: null, parent: null, before: null } });
  for (let c = 0; c < 8; c++) {
    const clip = ulid(3, c);
    await send(page, { domain: "Clip", command: { type: "CreateMidi", id: clip, track: first, start: c * 4, length: 4, name: null } });
    await send(page, {
      domain: "Note",
      command: {
        type: "Add",
        clip,
        notes: Array.from({ length: 16 }, (_, k) => ({ id: ulid(4, c * 16 + k), pitch: 60 + (k % 12), velocity: 100, start: k / 4, duration: 0.25 })),
      },
    });
  }
  for (let t = 1; t < tracks; t++) await send(page, { domain: "Track", command: { type: "Duplicate", id: first, new_id: ulid(1, t) } });
  await expect.poll(async () => Object.keys((await doc(page)).tracks).length).toBeGreaterThanOrEqual(tracks);
  await expect(page.locator(".eth-arr-row")).not.toHaveCount(0);
}

/** Sends `commands` as `Edit::Batch`es of `chunk` commands (one engine round trip each). */
export async function sendBatched(page: Page, commands: Command[], chunk = 400): Promise<void> {
  for (let i = 0; i < commands.length; i += chunk) {
    await send(page, { domain: "Edit", command: { type: "Batch", label: "fixture", commands: commands.slice(i, i + chunk) } });
  }
}

/**
 * The WebGL arranger fixture: `tracks` tracks (alternating audio and MIDI) with `clips` clips
 * in all, spread evenly: audio clips of the library's "Bass Loop 120.wav" (waveforms), MIDI
 * clips with 8 notes each, lengths 1–4 bars apart from a few short ones, every 7th clip looped
 * and every 11th muted, names "A12" / "M12". `importSample` puts the sample in the project
 * (a library drop on the given audio track) and resolves with its media id.
 */
export async function buildMixedProject(
  page: Page,
  opts: { tracks: number; clips: number; importSample: (audioTrack: string) => Promise<string> },
): Promise<{ audioTracks: string[]; midiTracks: string[] }> {
  await installProbe(page);
  const audioTracks: string[] = [];
  const midiTracks: string[] = [];
  const tracks: Command[] = [];
  for (let t = 0; t < opts.tracks; t++) {
    const kind = t % 2 === 0 ? "Audio" : "Midi";
    const id = ulid(5, t);
    (kind === "Audio" ? audioTracks : midiTracks).push(id);
    tracks.push({
      domain: "Track",
      command: { type: "Create", id, kind, name: `${kind === "Audio" ? "Audio" : "Keys"} ${t}`, color: null, parent: null, before: null },
    });
  }
  await sendBatched(page, tracks);
  const media = await opts.importSample(audioTracks[0]!);
  // The import's own clip goes: the fixture places its clips itself.
  const imported = Object.values((await doc(page)).clips).map((c) => c.id);
  if (imported.length) await send(page, { domain: "Clip", command: { type: "Delete", ids: imported } });
  const commands: Command[] = [];
  const perTrack = Math.ceil(opts.clips / opts.tracks);
  const lengths = [8, 4, 16, 2, 12, 1, 6, 4, 0.5];
  let n = 0;
  for (let t = 0; t < opts.tracks && n < opts.clips; t++) {
    const track = ulid(5, t);
    let at = (t % 5) * 2;
    for (let k = 0; k < perTrack && n < opts.clips; k++, n++) {
      const id = ulid(6, n);
      const length = lengths[(t + k) % lengths.length]!;
      if (t % 2 === 0) {
        commands.push({ domain: "Clip", command: { type: "CreateAudio", id, track, start: at, media } });
        commands.push({ domain: "Clip", command: { type: "SetBounds", id, start: at, length, offset: 0 } });
        commands.push({ domain: "Clip", command: { type: "Rename", id, name: `A${n}` } });
      } else {
        commands.push({ domain: "Clip", command: { type: "CreateMidi", id, track, start: at, length, name: `M${n}` } });
        commands.push({
          domain: "Note",
          command: {
            type: "Add",
            clip: id,
            notes: Array.from({ length: 8 }, (_, j) => ({
              id: ulid(7, n * 8 + j),
              pitch: 48 + ((j * 5 + n) % 24),
              velocity: 0.4 + (j % 4) * 0.2,
              start: (j * length) / 8,
              duration: length / 10,
            })),
          },
        });
      }
      if (n % 7 === 3)
        commands.push({ domain: "Clip", command: { type: "SetLoop", id, looping: { enabled: true, start: 0, end: Math.max(0.5, length / 2) } } });
      if (n % 11 === 5) commands.push({ domain: "Clip", command: { type: "SetMuted", ids: [id], muted: true } });
      at += length + ((t + k) % 3) * 0.5;
    }
  }
  await sendBatched(page, commands);
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 60_000 }).toBe(opts.clips);
  return { audioTracks, midiTracks };
}

/**
 * Frame timing is only meaningful when the browser gets the CPU it asks for. On a shared box
 * with more runnable threads than cores (1-min load average above the core count, or above
 * `maxLoad` when given) the timing assertions would measure the host, not the app: they are
 * logged and annotated instead. `ETHER_E2E_STRICT_PERF=1` asserts the timing anyway.
 */
export function hostHasHeadroom(maxLoad = Infinity): { ok: boolean; why: string } {
  const load = loadavg()[0]!;
  const cores = availableParallelism();
  const limit = Math.min(cores, maxLoad);
  const why = `1-min load ${load.toFixed(1)} on ${cores} cores (limit ${limit})`;
  return { ok: process.env.ETHER_E2E_STRICT_PERF === "1" || load <= limit, why };
}

/** Intervals (ms) of the next `n` animation frames. */
export const idleFrames = (page: Page, n = 40): Promise<number[]> =>
  page.evaluate(
    (n) =>
      new Promise<number[]>((done) => {
        const out: number[] = [];
        let last = performance.now();
        const tick = (now: number) => {
          out.push(now - last);
          last = now;
          if (out.length < n) requestAnimationFrame(tick);
          else done(out.slice(1));
        };
        requestAnimationFrame(tick);
      }),
    n,
  );

/** Start recording frame intervals; `stopFrames` returns them. */
export async function startFrames(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __rec: boolean };
    w.__frames = [];
    w.__rec = true;
    let last = performance.now();
    const tick = (now: number) => {
      w.__frames.push(now - last);
      last = now;
      if (w.__rec) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

export const stopFrames = (page: Page): Promise<number[]> =>
  page.evaluate(() => {
    const w = window as unknown as { __frames: number[]; __rec: boolean };
    w.__rec = false;
    return w.__frames.slice(1);
  });

export const median = (xs: number[]): number => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]!;
export const p95 = (xs: number[]): number => [...xs].sort((a, b) => a - b)[Math.floor(xs.length * 0.95)]!;

/** Main-thread metrics (CDP `Performance`), cumulative seconds / counts. */
export async function metrics(cdp: CDPSession) {
  const { metrics: m } = await cdp.send("Performance.getMetrics");
  const get = (n: string) => m.find((x) => x.name === n)?.value ?? 0;
  return {
    script: get("ScriptDuration"),
    layout: get("LayoutDuration") + get("RecalcStyleDuration"),
    task: get("TaskDuration"),
    layouts: get("LayoutCount"),
    styles: get("RecalcStyleCount"),
  };
}

/** Number of compositor layers of the page now (CDP `LayerTree`). */
export async function layerCount(cdp: CDPSession, page: Page): Promise<number> {
  const tree = new Promise<{ layerId: string }[]>((resolve) => {
    cdp.once("LayerTree.layerTreeDidChange", (e) => resolve(e.layers ?? []));
  });
  await cdp.send("LayerTree.enable");
  // Force a fresh commit so the event fires.
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  const layers = await tree;
  if (process.env.ETHER_E2E_LAYERS === "1") await logReasons(cdp, layers);
  await cdp.send("LayerTree.disable");
  return layers.length;
}

/** Histogram of the compositing reasons (`ETHER_E2E_LAYERS=1`, diagnostics). */
async function logReasons(cdp: CDPSession, layers: { layerId: string }[]): Promise<void> {
  const counts = new Map<string, number>();
  for (const l of layers.slice(0, 800)) {
    try {
      const r = await cdp.send("LayerTree.compositingReasons", { layerId: l.layerId });
      const k = (r.compositingReasons ?? []).join("+") || "(none)";
      counts.set(k, (counts.get(k) ?? 0) + 1);
    } catch {
      counts.set("(gone)", (counts.get("(gone)") ?? 0) + 1);
    }
  }
  console.log("layer reasons:", JSON.stringify([...counts].sort((a, b) => b[1] - a[1])));
}
