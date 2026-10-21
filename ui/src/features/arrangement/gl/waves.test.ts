// Audio clip waveforms: peak tiles shaped exactly like the engine's replies
// (`ether-media` `PeakMipmap::query`, ported below) through `LaneScene`, and the WebGL
// upload of the peak texture rows.
import { describe, expect, it } from "vitest";
import type { Clip, Command, MediaRef, PeakData, PeakRequest, Project, ReplyValue, Track } from "@/generated";
import { TempoMap } from "@/timeline";
import type { EngineTransport } from "@/transport";
import { createDemoProject } from "@/transport/mock/demoProject";
import type { Row } from "../layout";
import { PeakCache, TILE_PEAKS } from "../peaks";
import type { FrameSetup } from "./backend";
import { Batch } from "./glContext";
import { GlyphAtlas } from "./glyphAtlas";
import type { Palette } from "./palette";
import { InstanceWriter, notePipeline, rectPipeline, textPipeline, WAVE_STRIDE, wavePipeline, writeWave } from "./pipelines";
import { LaneScene, type SceneInput, type Shared, type ViewWindow } from "./scene";
import type { RasterRun, TextRasterizer } from "./textRaster";
import { TILE_ROWS, WaveTiles } from "./waveTiles";
import { WebglBackend } from "./webglBackend";

// ── The engine's peak pyramid (crates/ether-media/src/peaks.rs) ─────────────────────────

const BASE_SPP = 32;

/** Level sizes of `PeakMipmap::build` for `frames` frames: 32 spp, halved until 1 peak. */
function engineLevels(frames: number): Array<{ spp: number; len: number }> {
  const levels = [{ spp: BASE_SPP, len: Math.ceil(frames / BASE_SPP) }];
  for (;;) {
    const prev = levels[levels.length - 1]!;
    if (prev.len <= 1 || prev.spp > 0xffffffff / 2) break;
    levels.push({ spp: prev.spp * 2, len: Math.ceil(prev.len / 2) });
  }
  return levels;
}

/** The fake audio's level: tells the media apart in the texture rows. */
const amplitude = (m: MediaRef) => (m.channels === 1 ? 0.25 : 0.5);

/** `PeakMipmap::query`: nearest level >= requested (else the coarsest), whole peaks. */
function engineQuery(media: MediaRef, r: PeakRequest): PeakData {
  const levels = engineLevels(media.frames);
  const wanted = Math.max(1, r.samples_per_peak);
  const level = levels.find((l) => l.spp >= wanted) ?? levels[levels.length - 1]!;
  const start = Number.isFinite(r.start_frame) ? r.start_frame : 0;
  const end = start + Math.max(0, Number.isFinite(r.frame_count) ? r.frame_count : 0);
  const i0 = Math.min(Math.max(0, Math.floor(start / level.spp)), level.len);
  const i1 = Math.min(Math.max(Math.ceil(end / level.spp), i0), level.len);
  const n = i1 - i0;
  const chans = Array.from({ length: media.channels }, () => 0);
  return {
    media: r.media,
    samples_per_peak: level.spp,
    start_frame: i0 * level.spp,
    min: chans.map(() => new Array(n).fill(-amplitude(media))),
    max: chans.map(() => new Array(n).fill(amplitude(media))),
  };
}

function engineTransport(project: () => Project): { transport: EngineTransport; requests: PeakRequest[] } {
  const requests: PeakRequest[] = [];
  const send = async (c: Command): Promise<ReplyValue> => {
    const r = (c.command as { request: PeakRequest }).request;
    requests.push(r);
    return { type: "Peaks", peaks: engineQuery(project().media[r.media]!, r) };
  };
  return { transport: { send, kind: "tauri" } as unknown as EngineTransport, requests };
}

// ── Fixtures ──────────────────────────────────────────────────────────────────────────

class FakeRaster implements TextRasterizer {
  reset(): void {}
  measure(text: string): number {
    return text.length * 6;
  }
  metrics(): { ascent: number; descent: number } {
    return { ascent: 10, descent: 2 };
  }
  draw(text: string): RasterRun {
    const width = text.length * 6;
    return { data: new Uint8Array(width * 12), width, height: 12, originX: 0, baseline: 10, advance: width };
  }
  icon(_paths: ReadonlyArray<string>, px: number): Uint8Array {
    return new Uint8Array(px * px);
  }
}

const palette = new Proxy({}, { get: () => [0.5, 0.5, 0.5, 1] }) as unknown as Palette;

/** The demo project with its drums clip playing all of `media` (no warp: 1 s = 2 beats). */
function projectWith(media: Partial<MediaRef>, clip: Partial<Clip> = {}, reversed = false): { project: Project; track: Track; clip: Clip; media: MediaRef } {
  const p = createDemoProject();
  const base = Object.values(p.clips).find((c) => c.content.type === "Audio")!;
  const m: MediaRef = { ...p.media[(base.content as { media: string }).media]!, ...media };
  const seconds = m.frames / m.sample_rate;
  const content = {
    ...(base.content as Extract<Clip["content"], { type: "Audio" }>),
    warp: { enabled: false, mode: "Repitch" as const, source_bpm: 120 },
    reversed,
  };
  const c: Clip = { ...base, start: 0, offset: 0, length: seconds * 2, content, ...clip };
  return {
    project: { ...p, media: { ...p.media, [m.id]: m }, clips: { ...p.clips, [c.id]: c }, warp_markers: {} },
    track: p.tracks[c.track]!,
    clip: c,
    media: m,
  };
}

function sceneFor(project: Project, tiles = new WaveTiles()) {
  const { transport, requests } = engineTransport(() => project);
  const peaks = new PeakCache(transport);
  let gen = 0;
  peaks.subscribe(() => gen++);
  const shared: Shared = {
    pipelines: { rect: rectPipeline(), text: textPipeline(), note: notePipeline(), wave: wavePipeline() },
    atlas: new GlyphAtlas(new FakeRaster(), { width: 256, height: 64 }),
    tiles,
    peaks,
    peaksGeneration: () => gen,
    titleReserves: () => [],
    titleVersion: () => 0,
  };
  const input: SceneInput = {
    project,
    preview: null,
    selected: new Set(),
    hover: null,
    renaming: null,
    palette,
    tempo: TempoMap.constant(120),
    transportKind: "tauri",
  };
  return { shared, input, requests };
}

const settle = async () => {
  for (let i = 0; i < 4; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => requestAnimationFrame(r));
};

type Wave = { t0: number; t1: number; u0: number; u1: number; row: number };

function wavesOf(b: Batch): Wave[] {
  const out: Wave[] = [];
  for (let i = 0; i < b.count; i++) {
    const o = i * WAVE_STRIDE;
    const d = b.data;
    out.push({ t0: d[o]!, t1: d[o + 1]!, u0: d[o + 2]!, u1: d[o + 3]!, row: d[o + 4]! });
  }
  return out;
}

/** Builds the lane's view at `ppb` (peaks fetched, then rebuilt), returns its wave instances. */
async function wavesAt(lane: LaneScene, input: SceneInput, track: Track, ppb: number, from: number, width: number): Promise<Wave[]> {
  const span = width / ppb;
  const win: ViewWindow = { ppb, from: from - span, to: from + 2 * span, dpr: 2 };
  lane.sync(track, input);
  lane.syncView(input, win, from, from + span);
  await settle();
  lane.sync(track, input);
  lane.syncView(input, win, from, from + span);
  return wavesOf(lane.waves.base);
}

// ── Tests ─────────────────────────────────────────────────────────────────────────────

describe("audio clip waveforms with the engine's peak replies", () => {
  const cases: Array<{ name: string; media: Partial<MediaRef> }> = [
    { name: "3 min 48 kHz stereo", media: { sample_rate: 48000, channels: 2, frames: 48000 * 180 } },
    { name: "10 min 96 kHz mono", media: { sample_rate: 96000, channels: 1, frames: 96000 * 600 + 17 } },
    { name: "short one-shot (3.5k frames)", media: { sample_rate: 44100, channels: 1, frames: 3531 } },
    { name: "2 s loop", media: { sample_rate: 44100, channels: 2, frames: 88200 } },
  ];
  // From 400 px/beat down to a whole long song in a few px.
  const zooms = [400, 100, 25, 6, 1.5, 0.4, 0.1, 0.025, 0.006];

  for (const c of cases.flatMap((c) => [c, { ...c, name: `${c.name}, reversed`, reversed: true }])) {
    it(`covers the whole clip at every zoom: ${c.name}`, async () => {
      const { project, track, clip, media } = projectWith(c.media, {}, "reversed" in c);
      const { shared, input } = sceneFor(project);
      const lane = new LaneScene(track, shared);
      for (const ppb of zooms) {
        const width = 1200;
        const waves = await wavesAt(lane, input, track, ppb, 0, width);
        const visibleEnd = Math.min(clip.length, (2 * width) / ppb);
        expect(waves.length, `ppb ${ppb}`).toBeGreaterThan(0);
        // The instances tile [0, visibleEnd] without gaps.
        const spans = waves.map((w) => [w.t0, w.t1] as const).sort((a, b) => a[0] - b[0]);
        let reach = 0;
        for (const [a, b] of spans) {
          expect(a, `gap before ${a} at ppb ${ppb}`).toBeLessThanOrEqual(reach + 1e-6);
          reach = Math.max(reach, b);
        }
        expect(reach, `ppb ${ppb}`).toBeGreaterThanOrEqual(visibleEnd - 1e-6);
        // Each instance reads peaks of its row that exist (non-empty) over its whole span.
        for (const w of waves) {
          const data = shared.tiles.rowData(w.row)!;
          const lo = Math.max(0, Math.floor(Math.min(w.u0, w.u1)));
          const hi = Math.min(TILE_PEAKS - 1, Math.ceil(Math.max(w.u0, w.u1)) - 1);
          expect(lo, `ppb ${ppb}`).toBeLessThanOrEqual(hi);
          for (const i of [lo, Math.floor((lo + hi) / 2), hi]) {
            expect(data[i * 2 + 1]!, `peak ${i} of row ${w.row} at ppb ${ppb} (${media.frames} frames)`).toBeGreaterThanOrEqual(data[i * 2]!);
          }
        }
      }
    });
  }
});

describe("a compact lane", () => {
  it("draws no waveform: its clips are a title band alone", async () => {
    const { project, track } = projectWith({ sample_rate: 48000, channels: 2, frames: 48000 * 10 });
    const { shared, input } = sceneFor(project);
    const lane = new LaneScene(track, shared);
    expect((await wavesAt(lane, input, track, 100, 0, 1200)).length).toBeGreaterThan(0);
    lane.sync(track, input, 36);
    expect(lane.compact).toBe(true);
    expect(await wavesAt(lane, input, track, 100, 0, 1200)).toEqual([]);
    lane.sync(track, input, 64);
    expect((await wavesAt(lane, input, track, 100, 0, 1200)).length).toBeGreaterThan(0);
  });
});

// ── WebGL upload of the peak rows ─────────────────────────────────────────────────────

/** A WebGL2 context recording `texSubImage2D` rows (everything else is a no-op). */
function fakeGl(): { canvas: HTMLCanvasElement; uploads: number[] } {
  const uploads: number[] = [];
  const gl = new Proxy(
    {},
    {
      get(_t, key: string) {
        if (key === "getShaderParameter" || key === "getProgramParameter") return () => true;
        if (key === "isContextLost") return () => false;
        if (key === "texSubImage2D") return (...a: unknown[]) => uploads.push(a[3] as number);
        if (/^[A-Z0-9_]+$/.test(key)) return 1;
        return () => ({});
      },
    },
  );
  const canvas = { getContext: () => gl, addEventListener() {}, removeEventListener() {} } as unknown as HTMLCanvasElement;
  return { canvas, uploads };
}

const tile = (start: number): PeakData => ({ media: "m", samples_per_peak: 32, start_frame: start, min: [[-0.5]], max: [[0.5]] });
const frame: FrameSetup = {
  width: 400,
  height: 200,
  cssWidth: 400,
  cssHeight: 200,
  dpr: 1,
  scrollBeats: 0,
  pxPerBeat: 20,
  originY: 0,
  originX: 0,
  palette: { bg: [0, 0, 0, 1], over: [1, 0, 0, 1] } as unknown as Palette,
};
const row: Row = { track: { id: "t" } as Track, depth: 0, y: 10, laneHeight: 56, height: 56 };

describe("peak texture rows on the GPU", () => {
  it("uploads the rows a lane assigned after another lane's waveforms were drawn in the same frame", () => {
    const { canvas, uploads } = fakeGl();
    const tiles = new WaveTiles();
    const pipelines = { rect: rectPipeline(), text: textPipeline(), note: notePipeline(), wave: wavePipeline() };
    const backend = new WebglBackend(canvas, pipelines, new GlyphAtlas(new FakeRaster(), { width: 64, height: 64 }), tiles);
    const waves = (r: number) => {
      const w = new InstanceWriter(WAVE_STRIDE);
      writeWave(w, 0, 1, 0, 1, r, 1, 0, 1, { b0: 0, b1: 1, minW: 0 }, [1, 1, 1, 1]);
      const b = new Batch(pipelines.wave);
      b.set(w.take());
      return b;
    };
    backend.begin(frame);
    // Lane 1 builds its view (a row), then draws its waveform.
    tiles.tick();
    const a = tiles.row(tile(0));
    expect(backend.draw(pipelines.wave, waves(a), row, 0, 1)).toBe(true);
    expect(uploads).toEqual([a]);
    // Lane 2 builds its view later in the same frame (lanes are synced as they are drawn).
    const b = tiles.row(tile(32 * TILE_PEAKS));
    expect(backend.draw(pipelines.wave, waves(b), row, 0, 1)).toBe(true);
    // Its row holds its peaks before it's drawn (not the zeros / another tile's peaks).
    expect(uploads).toEqual([a, b]);
    backend.end();
  });

  it("doesn't recycle the rows of a lane drawn earlier in the frame for a later lane", async () => {
    // Lane 1: a 2 s stereo loop (6 tiles). Lane 2: a 10 min mono file zoomed in, its view
    // window needing more tiles than the texture has rows.
    const one = projectWith({ sample_rate: 44100, channels: 2, frames: 88200 });
    const p = one.project;
    const other =
      Object.values(p.tracks).find((t) => t.id !== one.track.id && t.kind === "Audio") ?? Object.values(p.tracks).find((t) => t.id !== one.track.id)!;
    const longMedia: MediaRef = { ...one.media, id: "long", channels: 1, sample_rate: 48000, frames: 48000 * 600 };
    const longClip: Clip = {
      ...one.clip,
      id: "long-clip",
      track: other.id,
      length: 1200,
      content: { ...(one.clip.content as Extract<Clip["content"], { type: "Audio" }>), media: "long" },
    };
    const project: Project = { ...p, media: { ...p.media, long: longMedia }, clips: { ...p.clips, [longClip.id]: longClip } };
    const { shared, input } = sceneFor(project);
    const lane1 = new LaneScene(one.track, shared);
    const lane2 = new LaneScene(other, shared);
    // 750 px/beat (2 dpr): 16 frames per device px (16 Ki-frame tiles); 3 × 100000 px of
    // window: ~600 tiles.
    const width = 100000;
    const ppb = 750;
    const win: ViewWindow = { ppb, from: 0, to: (3 * width) / ppb, dpr: 2 };
    const frame = () => {
      // As the renderer draws a frame: one tick, then each lane syncs as it's drawn.
      shared.tiles.tick();
      for (const [lane, track] of [
        [lane1, one.track],
        [lane2, other],
      ] as const) {
        lane.sync(track, input);
        lane.syncView(input, win, 0, width / ppb);
        shared.tiles.touch(lane.waveRows);
      }
    };
    frame();
    await settle();
    frame();
    const rows = lane1.waveRows;
    expect(rows.length).toBe(6);
    expect(lane2.waveRows.length).toBe(TILE_ROWS - 6);
    // Lane 1's rows still hold the loop's peaks (not the long file's).
    for (const r of rows) expect(shared.tiles.rowData(r)![1]).toBe(0.5);
  });
});
