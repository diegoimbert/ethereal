/**
 * Pure layout of ruler ticks and labels for a viewport (bars/beats or minutes:seconds).
 * `Ruler` renders these; views drawing their own ruler (canvas) can reuse them.
 */

import { adaptiveStep, gridLines } from "./grid";
import { formatBarBeat, formatSeconds } from "./format";
import { beatsPerBar, beatUnit, type TempoMap } from "./tempoMap";
import { beatsToPx, visibleRange, type TimelineViewport } from "./viewport";

export type RulerFormat = "bars" | "seconds";

export interface RulerTick {
  x: number;
  major: boolean;
}

export interface RulerLabel {
  x: number;
  text: string;
}

export interface RulerMarks {
  ticks: RulerTick[];
  labels: RulerLabel[];
}

/** Minimum pixels between labels. */
export const RULER_LABEL_MIN_PX = 44;
/** Minimum pixels between ticks. */
export const RULER_TICK_MIN_PX = 8;

/** "Nice" second intervals for the seconds ruler. */
const SECOND_STEPS = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800, 3600];

export function rulerMarks(tempo: TempoMap, vp: TimelineViewport, widthPx: number, format: RulerFormat): RulerMarks {
  return format === "bars" ? barMarks(tempo, vp, widthPx) : secondMarks(tempo, vp, widthPx);
}

function barMarks(tempo: TempoMap, vp: TimelineViewport, widthPx: number): RulerMarks {
  const range = visibleRange(vp, widthPx);
  const sig = tempo.signatureAt(range.start);
  const tickStep = adaptiveStep(vp.pxPerBeat, RULER_TICK_MIN_PX, sig);
  const ticks: RulerTick[] = gridLines(tempo, range, tickStep).map((l) => ({
    x: beatsToPx(l.beats, vp),
    major: l.level === "bar",
  }));

  const labels: RulerLabel[] = [];
  const barPx = beatsPerBar(sig) * vp.pxPerBeat;
  const beatPx = beatUnit(sig) * vp.pxPerBeat;
  if (beatPx >= RULER_LABEL_MIN_PX) {
    // Label every beat as "bar.beat" (bar lines as just "bar").
    for (const l of gridLines(tempo, range, { kind: "beats", beats: beatUnit(sig) })) {
      if (l.level === "sub") continue;
      labels.push({ x: beatsToPx(l.beats, vp), text: formatBarBeat(l.beats, tempo, l.level === "bar" ? 1 : 2) });
    }
  } else {
    let every = 1;
    while (every * barPx < RULER_LABEL_MIN_PX && every < 1 << 16) every *= 2;
    for (const b of tempo.barLines(range.start, range.end, every)) {
      labels.push({ x: beatsToPx(b.beats, vp), text: String(b.bar) });
    }
  }
  return { ticks, labels };
}

function secondMarks(tempo: TempoMap, vp: TimelineViewport, widthPx: number): RulerMarks {
  const range = visibleRange(vp, widthPx);
  const s0 = tempo.beatsToSeconds(range.start);
  const s1 = tempo.beatsToSeconds(range.end);
  const pxPerSecond = widthPx / Math.max(s1 - s0, 1e-9);
  const step = SECOND_STEPS.find((s) => s * pxPerSecond >= RULER_LABEL_MIN_PX * 1.5) ?? 3600;
  const minor = step / (step * pxPerSecond / 5 >= RULER_TICK_MIN_PX ? 5 : 2);
  const ticks: RulerTick[] = [];
  const labels: RulerLabel[] = [];
  const first = Math.ceil(s0 / minor - 1e-9);
  for (let i = first; i * minor <= s1 + 1e-9 && ticks.length < 5000; i++) {
    const t = i * minor;
    const x = beatsToPx(tempo.secondsToBeats(t), vp);
    const major = Math.abs(t / step - Math.round(t / step)) < 1e-6;
    ticks.push({ x, major });
    if (major) labels.push({ x, text: formatSeconds(t, step < 1) });
  }
  return { ticks, labels };
}

