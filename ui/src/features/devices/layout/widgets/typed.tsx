/**
 * Typed widgets of the catalog (bind several params): Envelope, FilterCurve, TransferCurve,
 * Oscillator, Lfo, StepEditor, XyPad, Crossover. Each is a graphic (draggable where the
 * contract says so) over a row of small controls for the params it binds, so a layout never
 * repeats them; every one of those controls is MIDI-learnable and a modulation target.
 */

import clsx from "clsx";
import { useRef, type ReactNode } from "react";
import type { ParamId, ParamInfo, Widget, WidgetSize } from "@/generated";
import { labelIndex } from "../../paramScale";
import { bindParam, useLayoutContext, useParam, type ParamBinding } from "../context";
import {
  filterMagnitudeDb,
  filterShapeOf,
  freqToX,
  resonanceToQ,
  samplePath,
  transfer,
  waveAt,
  waveOf,
  xToFreq,
} from "../curves";
import { genericWidget } from "../model";
import { GridLines, Plot } from "../plot";
import { WidgetView } from "../Widget";

type Of<T extends Widget["type"]> = Extract<Widget, { type: T }>;

export interface TypedProps<T extends Widget["type"]> {
  widget: Of<T>;
  size: WidgetSize;
  label: string | null;
}

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);
/** Handle hit radius in px (pointer tolerance, not a visual size). */
const HIT = 14;

/** Catalog name of a kebab widget kind ("filter-curve" → "FilterCurve"). */
function catalogName(kind: string): string {
  return kind
    .split("-")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join("");
}

/** Frame of a typed widget: optional caption, the graphic and its controls row. */
export function TypedFrame({
  type,
  size,
  label,
  children,
  controls,
}: {
  type: string;
  size: WidgetSize;
  label: string | null;
  children: ReactNode;
  controls?: ReadonlyArray<ParamId | null | undefined>;
}) {
  return (
    <div className={clsx("eth-widget", `eth-widget--${type}`, `eth-widget--${size.toLowerCase()}`)} data-widget={catalogName(type)}>
      {label && <span className="eth-widget__label">{label}</span>}
      {children}
      {controls && <ControlsRow ids={controls} size={size === "Large" ? "Medium" : "Small"} />}
    </div>
  );
}

/** Generic controls (knob / toggle / choice) for the given params: small, medium under a large graphic. */
export function ControlsRow({ ids, size = "Small" }: { ids: ReadonlyArray<ParamId | null | undefined>; size?: WidgetSize }) {
  const ctx = useLayoutContext();
  const infos = ids.flatMap((id) => {
    const info = id == null ? undefined : ctx.params.get(id);
    return info ? [info] : [];
  });
  if (infos.length === 0) return null;
  return (
    <div className="eth-widget__controls">
      {infos.map((info) => (
        <WidgetView key={info.id} widget={genericWidget(info)} size={size} label={info.name} />
      ))}
    </div>
  );
}

/** Label of an enum param's current value (for shape/mode lookups). */
function currentLabel(b: ParamBinding | null): string | null {
  if (!b || !b.info.labels) return null;
  return b.info.labels[labelIndex(b.info, b.plain)] ?? null;
}

/** 0..1 position of a frequency param on the log axis (Hz), else its normalized value. */
function freqX(b: ParamBinding): number {
  return b.info.unit === "Hertz" ? freqToX(b.plain) : b.normalized;
}

/** Set a frequency param from a 0..1 log-axis position. */
function setFreqX(b: ParamBinding, x: number): void {
  if (b.info.unit === "Hertz") b.setPlain(xToFreq(x));
  else b.setNormalized(clamp01(x));
}

// ---- Envelope --------------------------------------------------------------------------

interface EnvGeometry {
  unit: number;
  top: number;
  bottom: number;
  pts: Array<[number, number]>;
  handles: Array<{ b: ParamBinding; x: number; y: number; x0: number; vertical?: boolean }>;
}

/** Width of the sustain plateau, in time-segment units. */
const SUSTAIN_UNITS = 0.75;

export function EnvelopeWidget({ widget: w, size, label }: TypedProps<"Envelope">) {
  const delay = useParam(w.delay);
  const attack = useParam(w.attack);
  const hold = useParam(w.hold);
  const decay = useParam(w.decay);
  const sustain = useParam(w.sustain);
  const release = useParam(w.release);
  const ctx = useLayoutContext();
  const grab = useRef<{ b: ParamBinding; x0: number; vertical?: boolean } | null>(null);
  const geom = useRef<EnvGeometry | null>(null);
  if (!attack || !decay || !sustain || !release) return <Missing type="Envelope" />;
  const segs = [delay, attack, hold, decay].filter((b): b is ParamBinding => b !== null);
  const units = segs.length + 1 + SUSTAIN_UNITS;
  const geometry = (wpx: number, h: number): EnvGeometry => {
    const unit = wpx / units;
    const top = h * 0.08;
    const bottom = h - 1;
    const level = bottom - sustain.normalized * (bottom - top);
    let x = 0;
    const pts: Array<[number, number]> = [[0, bottom]];
    const handles: Array<{ b: ParamBinding; x: number; y: number; x0: number; vertical?: boolean }> = [];
    if (delay) {
      const x0 = x;
      x += delay.normalized * unit;
      pts.push([x, bottom]);
      handles.push({ b: delay, x, y: bottom, x0 });
    }
    {
      const x0 = x;
      x += attack.normalized * unit;
      pts.push([x, top]);
      handles.push({ b: attack, x, y: top, x0 });
    }
    if (hold) {
      const x0 = x;
      x += hold.normalized * unit;
      pts.push([x, top]);
      handles.push({ b: hold, x, y: top, x0 });
    }
    const decayX0 = x;
    x += decay.normalized * unit;
    pts.push([x, level]);
    handles.push({ b: decay, x, y: level, x0: decayX0, vertical: true });
    x += SUSTAIN_UNITS * unit;
    pts.push([x, level]);
    const relX0 = x;
    x += release.normalized * unit;
    pts.push([x, bottom]);
    handles.push({ b: release, x, y: bottom, x0: relX0 });
    return { unit, top, bottom, pts, handles };
  };
  return (
    <TypedFrame type="envelope" size={size} label={label} controls={[w.delay, w.attack, w.hold, w.decay, w.sustain, w.release]}>
      <Plot
        className="eth-plot--envelope"
        label="Envelope"
        testId="widget-envelope"
        begin={ctx.sender.begin}
        end={ctx.sender.end}
        drag={{
          start: (x, y) => {
            const g = geom.current;
            if (!g) return false;
            let best: EnvGeometry["handles"][number] | null = null;
            let dist = HIT;
            for (const hd of g.handles) {
              const d = Math.hypot(hd.x - x, hd.y - y);
              if (d <= dist) {
                dist = d;
                best = hd;
              }
            }
            if (!best) return false;
            grab.current = { b: best.b, x0: best.x0, vertical: best.vertical };
          },
          move: (x, y) => {
            const gr = grab.current;
            const g = geom.current;
            if (!gr || !g) return;
            // Re-bind by id: the binding captured at press time may be stale after renders.
            const b = [delay, attack, hold, decay, release].find((c) => c?.info.id === gr.b.info.id) ?? gr.b;
            b.setNormalized(clamp01((x - gr.x0) / g.unit));
            if (gr.vertical) sustain.setNormalized(clamp01((g.bottom - y) / (g.bottom - g.top)));
          },
          end: () => {
            grab.current = null;
          },
        }}
      >
        {({ w: wpx, h }) => {
          const g = geometry(wpx, h);
          geom.current = g;
          const line = g.pts.map(([x, y]) => `${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.5]} />
              <polygon className="eth-plot__fill" points={`${line} ${g.pts[g.pts.length - 1]![0]},${g.bottom} 0,${g.bottom}`} />
              <polyline className="eth-plot__line" points={line} />
              {g.handles.map((hd) => (
                <circle key={hd.b.info.id} className="eth-plot__handle" cx={hd.x} cy={hd.y} r={4} data-handle={hd.b.info.name} />
              ))}
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

// ---- Filter curve ----------------------------------------------------------------------

/** dB range of the filter plot (±). */
const FILTER_DB = 24;

export function FilterCurveWidget({ widget: w, size, label }: TypedProps<"FilterCurve">) {
  const cutoff = useParam(w.cutoff);
  const resonance = useParam(w.resonance);
  const mode = useParam(w.mode);
  const gain = useParam(w.gain);
  const ctx = useLayoutContext();
  if (!cutoff || !resonance) return <Missing type="FilterCurve" />;
  const shape = filterShapeOf(currentLabel(mode));
  const fcX = freqX(cutoff);
  const fc = xToFreq(fcX);
  const q = resonanceToQ(resonance.normalized);
  const g = gain ? (gain.info.unit === "Decibels" ? gain.plain : (gain.normalized * 2 - 1) * FILTER_DB) : 0;
  const yOf = (db: number, h: number) => h / 2 - (Math.max(-FILTER_DB * 2, Math.min(FILTER_DB, db)) / FILTER_DB) * (h / 2) * 0.9;
  return (
    <TypedFrame type="filter-curve" size={size} label={label} controls={[w.mode, w.cutoff, w.resonance, w.drive, w.gain]}>
      <Plot
        className="eth-plot--filter"
        label={`Filter response, cutoff ${cutoff.text}`}
        testId="widget-filter-curve"
        begin={ctx.sender.begin}
        end={ctx.sender.end}
        onDoubleClick={() => {
          cutoff.reset();
          resonance.reset();
        }}
        drag={{
          start: () => true,
          move: (x, y, e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setFreqX(cutoff, clamp01(x / (r.width || 1)));
            resonance.setNormalized(clamp01(1 - y / (r.height || 1)));
          },
        }}
      >
        {({ w: wpx, h }) => {
          const path = samplePath(96, wpx, (t) => yOf(filterMagnitudeDb(shape, xToFreq(t), fc, q, g), h));
          const hy = yOf(filterMagnitudeDb(shape, fc, fc, q, g), h);
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.5]} xs={[freqToX(100), freqToX(1000), freqToX(10000)]} />
              <polygon className="eth-plot__fill" points={`0,${h} ${path} ${wpx},${h}`} />
              <polyline className="eth-plot__line" points={path} />
              <circle className="eth-plot__handle" cx={fcX * wpx} cy={Math.max(4, Math.min(h - 4, hy))} r={4} />
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

// ---- Transfer curve --------------------------------------------------------------------

function driveGain(b: ParamBinding): number {
  return b.info.unit === "Decibels" ? Math.pow(10, Math.max(0, b.plain) / 20) : 1 + 19 * b.normalized;
}

export function TransferCurveWidget({ widget: w, size, label }: TypedProps<"TransferCurve">) {
  const drive = useParam(w.drive);
  const curve = useParam(w.curve);
  const bias = useParam(w.bias);
  if (!drive) return <Missing type="TransferCurve" />;
  const k = driveGain(drive);
  const c = curve ? curve.normalized : 0;
  const bi = bias ? bias.normalized * 2 - 1 : 0;
  return (
    <TypedFrame type="transfer-curve" size={size} label={label} controls={[w.drive, w.curve, w.bias]}>
      <Plot className="eth-plot--square" label="Transfer curve" testId="widget-transfer-curve">
        {({ w: wpx, h }) => {
          const path = samplePath(64, wpx, (t) => h / 2 - (transfer(t * 2 - 1, k, c, bi) * h) / 2);
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.5]} xs={[0.5]} />
              <line className="eth-plot__ref" x1={0} y1={h} x2={wpx} y2={0} />
              <polyline className="eth-plot__line" points={path} />
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

// ---- Oscillator / LFO ------------------------------------------------------------------

function WavePlot({ kind, morph, cycles, amp, label, testId }: { kind: ReturnType<typeof waveOf>; morph: number; cycles: number; amp: number; label: string; testId: string }) {
  return (
    <Plot className="eth-plot--wave" label={label} testId={testId}>
      {({ w: wpx, h }) => {
        const path = samplePath(Math.max(64, Math.round(wpx)), wpx, (t) => h / 2 - waveAt(kind, t * cycles, morph) * amp * (h / 2) * 0.85);
        return (
          <>
            <GridLines w={wpx} h={h} ys={[0.5]} />
            <polyline className="eth-plot__line" points={path} />
          </>
        );
      }}
    </Plot>
  );
}

/** Wave kind of a shape param: its label, or a wavetable morph when it is continuous. */
function shapeKind(b: ParamBinding): ReturnType<typeof waveOf> {
  return b.info.labels ? waveOf(currentLabel(b)) : "table";
}

export function OscillatorWidget({ widget: w, size, label }: TypedProps<"Oscillator">) {
  const shape = useParam(w.shape);
  const position = useParam(w.position);
  if (!shape) return <Missing type="Oscillator" />;
  const kind = shapeKind(shape);
  const morph = position ? position.normalized : shape.info.labels ? 0.5 : shape.normalized;
  return (
    <TypedFrame type="oscillator" size={size} label={label} controls={[w.shape, w.position]}>
      <WavePlot kind={kind} morph={morph} cycles={1} amp={1} label={`Oscillator: ${shape.text}`} testId="widget-oscillator" />
    </TypedFrame>
  );
}

export function LfoWidget({ widget: w, size, label }: TypedProps<"Lfo">) {
  const shape = useParam(w.shape);
  const rate = useParam(w.rate);
  const amount = useParam(w.amount);
  if (!shape || !rate) return <Missing type="Lfo" />;
  const kind = shapeKind(shape);
  return (
    <TypedFrame type="lfo" size={size} label={label} controls={[w.shape, w.rate, w.amount]}>
      <WavePlot
        kind={kind}
        morph={shape.info.labels ? 0.5 : shape.normalized}
        cycles={2}
        amp={amount ? Math.max(0.1, Math.abs(amount.normalized * (amount.bipolar ? 2 : 1) - (amount.bipolar ? 1 : 0))) : 1}
        label={`LFO: ${shape.text}, ${rate.text}`}
        testId="widget-lfo"
      />
    </TypedFrame>
  );
}

// ---- Step editor -----------------------------------------------------------------------

export function StepEditorWidget({ widget: w, size, label }: TypedProps<"StepEditor">) {
  const ctx = useLayoutContext();
  const steps: ParamBinding[] = [];
  for (let i = 0; i < w.count; i++) {
    const info: ParamInfo | undefined = ctx.params.get(w.first + i);
    if (info) steps.push(bindParam(ctx, info));
  }
  if (steps.length === 0) return <Missing type="StepEditor" />;
  const n = steps.length;
  const bipolar = steps[0]!.bipolar;
  const at = (x: number, y: number, r: DOMRect) => {
    const i = Math.max(0, Math.min(n - 1, Math.floor((x / (r.width || 1)) * n)));
    steps[i]!.setNormalized(clamp01(1 - y / (r.height || 1)));
  };
  return (
    <TypedFrame type="step-editor" size={size} label={label}>
      <Plot
        className="eth-plot--steps"
        label={`${n} steps`}
        testId="widget-step-editor"
        begin={ctx.sender.begin}
        end={ctx.sender.end}
        drag={{
          start: (x, y, e) => at(x, y, e.currentTarget.getBoundingClientRect()),
          move: (x, y, e) => at(x, y, e.currentTarget.getBoundingClientRect()),
        }}
      >
        {({ w: wpx, h }) => {
          const bw = wpx / n;
          const zero = bipolar ? h / 2 : h;
          return (
            <>
              {bipolar && <GridLines w={wpx} h={h} ys={[0.5]} />}
              {steps.map((s, i) => {
                const y = h - s.normalized * h;
                return (
                  <rect
                    key={s.info.id}
                    className="eth-plot__bar"
                    x={i * bw + 1}
                    width={Math.max(1, bw - 2)}
                    y={Math.min(y, zero)}
                    height={Math.max(1, Math.abs(zero - y))}
                    data-step={i}
                  >
                    <title>{`${s.info.name}: ${s.text}`}</title>
                  </rect>
                );
              })}
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

// ---- XY pad ----------------------------------------------------------------------------

export function XyPadWidget({ widget: w, size, label }: TypedProps<"XyPad">) {
  const x = useParam(w.x);
  const y = useParam(w.y);
  const ctx = useLayoutContext();
  if (!x || !y) return <Missing type="XyPad" />;
  const set = (px: number, py: number, r: DOMRect) => {
    x.setNormalized(clamp01(px / (r.width || 1)));
    y.setNormalized(clamp01(1 - py / (r.height || 1)));
  };
  return (
    <TypedFrame type="xy-pad" size={size} label={label} controls={[w.x, w.y]}>
      <Plot
        className="eth-plot--square eth-plot--xy"
        label={`${x.info.name} ${x.text}, ${y.info.name} ${y.text}`}
        testId="widget-xy-pad"
        begin={ctx.sender.begin}
        end={ctx.sender.end}
        onDoubleClick={() => {
          x.reset();
          y.reset();
        }}
        drag={{
          start: (px, py, e) => set(px, py, e.currentTarget.getBoundingClientRect()),
          move: (px, py, e) => set(px, py, e.currentTarget.getBoundingClientRect()),
        }}
      >
        {({ w: wpx, h }) => {
          const cx = x.normalized * wpx;
          const cy = (1 - y.normalized) * h;
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.5]} xs={[0.5]} />
              <line className="eth-plot__cross" x1={cx} x2={cx} y1={0} y2={h} />
              <line className="eth-plot__cross" x1={0} x2={wpx} y1={cy} y2={cy} />
              <circle className="eth-plot__handle eth-plot__handle--lg" cx={cx} cy={cy} r={6} />
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

// ---- Crossover -------------------------------------------------------------------------

export function CrossoverWidget({ widget: w, size, label }: TypedProps<"Crossover">) {
  const ctx = useLayoutContext();
  const grab = useRef<ParamId | null>(null);
  const freqs = w.frequencies.flatMap((id) => {
    const info = ctx.params.get(id);
    return info ? [bindParam(ctx, info)] : [];
  });
  if (freqs.length === 0) return <Missing type="Crossover" />;
  return (
    <TypedFrame type="crossover" size={size} label={label} controls={w.frequencies}>
      <Plot
        className="eth-plot--crossover"
        label={`Crossover: ${freqs.map((f) => f.text).join(", ")}`}
        testId="widget-crossover"
        begin={ctx.sender.begin}
        end={ctx.sender.end}
        drag={{
          start: (x, _y, e) => {
            const wpx = e.currentTarget.getBoundingClientRect().width || 1;
            let best: ParamBinding | null = null;
            let dist = HIT;
            for (const f of freqs) {
              const d = Math.abs(freqX(f) * wpx - x);
              if (d <= dist) {
                dist = d;
                best = f;
              }
            }
            if (!best) return false;
            grab.current = best.info.id;
          },
          move: (x, _y, e) => {
            const f = freqs.find((b) => b.info.id === grab.current);
            if (f) setFreqX(f, clamp01(x / (e.currentTarget.getBoundingClientRect().width || 1)));
          },
          end: () => {
            grab.current = null;
          },
        }}
      >
        {({ w: wpx, h }) => {
          const xs = freqs.map((f) => freqX(f) * wpx);
          const edges = [0, ...xs, wpx];
          return (
            <>
              <GridLines w={wpx} h={h} ys={[]} xs={[freqToX(100), freqToX(1000), freqToX(10000)]} />
              {edges.slice(0, -1).map((x0, i) => (
                <rect key={`b${i}`} className={clsx("eth-plot__band", i % 2 === 1 && "eth-plot__band--alt")} x={x0} y={0} width={Math.max(0, edges[i + 1]! - x0)} height={h} />
              ))}
              {freqs.map((f, i) => (
                <g key={f.info.id} className="eth-plot__split" data-handle={f.info.name}>
                  <line x1={xs[i]} x2={xs[i]} y1={0} y2={h} />
                  <circle className="eth-plot__handle" cx={xs[i]} cy={h / 2} r={4} />
                </g>
              ))}
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

/** A widget whose bound params are missing from the descriptor (a spec bug). */
export function Missing({ type }: { type: string }) {
  return (
    <div className="eth-widget eth-widget--missing" data-widget={type}>
      {type}: unknown param
    </div>
  );
}

