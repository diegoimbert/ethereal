/**
 * The interactive EQ curve (`Widget::EqCurve`, CONTRACTS.md §12.15): the summed response of
 * the enabled bands on a log-frequency / dB grid, one handle per band and the device's live
 * input/output spectrum behind it.
 *
 * Gestures on a handle: drag = frequency (x) + gain (y; x only for shapes without gain),
 * Alt-drag or wheel = Q, Shift = fine, double-click = band on/off, context menu = on/off,
 * type and reset. Every drag (and every wheel burst) is one undo step. Focused handles take
 * arrow keys (←/→ frequency, ↑/↓ gain, Alt+↑/↓ Q; Shift = fine).
 */

import clsx from "clsx";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import type { Widget, WidgetSize } from "@/generated";
import { openContextMenu } from "@/kit";
import { inputSettings, readWheel, wheelScrollPx } from "@/timeline";
import { labelIndex, labelValue } from "../../paramScale";
import { bindParam, useLayoutContext, type ParamBinding } from "../context";
import { freqToX, xToFreq } from "../curves";
import { Plot } from "../plot";
import { TypedFrame } from "../widgets/typed";
import { bandCurves, summedDb } from "./eqResponse";
import {
  DB_RANGE,
  DB_TICKS,
  FREQ_LABELS,
  FREQ_TICKS,
  HIT,
  freqLabel,
  handleAt,
  hitBand,
  logFreqs,
  pxPerDb,
  qAfter,
  resolveBand,
  spectrumY,
  usesGain,
  xOf,
  yOf,
  type Plot as PlotBox,
  type ResolvedBand,
} from "./geometry";
import { useEngineSampleRate, useEqSpectra } from "./hooks";
import "./eq.css";

type EqCurveSpec = Extract<Widget, { type: "EqCurve" }>;

export interface EqCurveProps {
  widget: EqCurveSpec;
  size: WidgetSize;
  label: string | null;
}

/** Curve sample points (log-spaced, so point `i` sits at `x = i / (N - 1) · width`). */
const POINTS = 256;
const FREQS = logFreqs(POINTS);
/** Wheel bursts closer than this share one undo step. */
const WHEEL_GESTURE_MS = 400;
/** Presses closer than this on one handle make a double-click. */
const DOUBLE_MS = 500;

const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

type Grab =
  | { type: "band"; index: number; mode: "move" | "q"; x: number; y: number; fx: number; gain: number; q: number }
  | { type: "crossover"; param: number };

function toggleOn(b: ResolvedBand): void {
  const on = b.bindings.on;
  if (on) on.setPlain(b.on ? on.info.min : on.info.max);
}

function setQ(b: ResolvedBand, q: number): void {
  b.bindings.q?.setPlain(q);
}

/** "Band 3 · Bell · 1.00 kHz · +3.0 dB · Q 0.71" (what the handle currently does). */
function readout(b: ResolvedBand): string {
  const parts = [`Band ${b.index + 1}`];
  const kind = b.bindings.kind;
  if (kind) parts.push(kind.text);
  parts.push(b.bindings.freq.text);
  if (b.bindings.gain && usesGain(b.shape)) parts.push(b.bindings.gain.text);
  if (b.bindings.q) parts.push(`Q ${b.bindings.q.text}`);
  if (!b.on) parts.push("off");
  return parts.join(" · ");
}

export function EqCurveWidget({ widget: w, size, label }: EqCurveProps) {
  const ctx = useLayoutContext();
  const { sender } = ctx;
  const sampleRate = useEngineSampleRate();
  const spectra = useEqSpectra(ctx.device.id, w.spectrum !== "None");
  const bind = (id: number | null): ParamBinding | null => {
    const info = id == null ? undefined : ctx.params.get(id);
    return info ? bindParam(ctx, info) : null;
  };
  const bands = w.bands.flatMap((spec, i) => {
    const b = resolveBand(i, spec, bind);
    return b ? [b] : [];
  });
  const crossovers = w.crossovers.flatMap((id) => {
    const b = bind(id);
    return b ? [b] : [];
  });

  const [selected, setSelected] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [dragging, setDragging] = useState(false);

  // Latest bindings and plot size for event handlers (renders replace them).
  const live = useRef({ bands, crossovers, plot: { w: 0, h: 0 } as PlotBox });
  useLayoutEffect(() => {
    live.current.bands = bands;
    live.current.crossovers = crossovers;
  });
  const grab = useRef<Grab | null>(null);
  const lastPress = useRef<{ index: number; at: number } | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  const wheel = useRef<{ timer: ReturnType<typeof setTimeout> | null }>({ timer: null });

  const endWheel = () => {
    if (wheel.current.timer === null) return;
    clearTimeout(wheel.current.timer);
    wheel.current.timer = null;
    sender.end();
  };

  // Response of each band at the sample points; recomputed when a band's settings change.
  const key = bands.map((b) => `${b.shape}:${b.freq}:${b.gainDb}:${b.q}:${b.on ? 1 : 0}`).join("|");
  const curves = useMemo(() => bandCurves(bands, FREQS, sampleRate), [key, sampleRate]); // eslint-disable-line react-hooks/exhaustive-deps
  const sum = useMemo(() => summedDb(curves, POINTS), [curves]);

  // Wheel = Q on the handle under the pointer (native listener: React's is passive).
  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      const svg = el.querySelector("svg");
      if (!svg) return;
      const r = svg.getBoundingClientRect();
      const { bands: bs, plot } = live.current;
      const hit = hitBand(plot, bs, e.clientX - r.left, e.clientY - r.top, null);
      const b = hit === null ? undefined : bs.find((x) => x.index === hit);
      if (!b?.bindings.q) return;
      e.preventDefault();
      if (wheel.current.timer === null) sender.begin();
      else clearTimeout(wheel.current.timer);
      wheel.current.timer = setTimeout(() => {
        wheel.current.timer = null;
        sender.end();
      }, WHEEL_GESTURE_MS);
      setSelected(b.index);
      const info = b.bindings.q.info;
      // Normalized like every wheel (Settings > Input: scroll sensitivity, vertical inversion).
      // Shift = fine; some platforms turn a shifted vertical wheel into a horizontal one.
      const n = readWheel(e, el);
      const d = wheelScrollPx(n.dy || n.dx, "y", inputSettings());
      setQ(b, clamp(qAfter(b.q, d * (e.shiftKey ? 0.05 : 0.25)), info.min, info.max));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      el.removeEventListener("wheel", onWheel);
      endWheel();
    };
  }, [sender]); // eslint-disable-line react-hooks/exhaustive-deps

  const localPoint = (e: { clientX: number; clientY: number }) => {
    const svg = wrap.current?.querySelector("svg");
    const r = svg?.getBoundingClientRect();
    return r ? { x: e.clientX - r.left, y: e.clientY - r.top } : null;
  };

  const onContextMenu = (e: MouseEvent) => {
    const pt = localPoint(e);
    const hit = pt ? hitBand(live.current.plot, bands, pt.x, pt.y, selected) : null;
    const b = hit === null ? undefined : bands.find((x) => x.index === hit);
    if (!b) {
      openContextMenu(e, []);
      return;
    }
    setSelected(b.index);
    const kind = b.bindings.kind;
    const current = kind ? labelIndex(kind.info, kind.plain) : -1;
    const reset = () => {
      sender.begin();
      for (const p of [b.bindings.freq, b.bindings.gain, b.bindings.q, b.bindings.kind]) p?.reset();
      sender.end();
    };
    openContextMenu(e, [
      ...(b.bindings.on ? [{ label: b.on ? "Disable Band" : "Enable Band", onSelect: () => toggleOn(b) }, "separator" as const] : []),
      ...(kind?.info.labels ?? []).map((l, i) => ({
        label: `${i === current ? "✓ " : ""}${l}`,
        onSelect: () => kind!.setPlain(labelValue(kind!.info, i)),
      })),
      ...(kind?.info.labels?.length ? ["separator" as const] : []),
      { label: "Reset Band", onSelect: reset },
    ]);
  };

  const onKeyDown = (b: ResolvedBand) => (e: KeyboardEvent) => {
    const fine = e.shiftKey ? 0.1 : 1;
    const { freq, gain, q } = b.bindings;
    switch (e.key) {
      case "ArrowLeft":
      case "ArrowRight": {
        const dir = e.key === "ArrowLeft" ? -1 : 1;
        freq.setPlain(xToFreq(freqToX(b.freq) + (dir * fine) / 60));
        break;
      }
      case "ArrowUp":
      case "ArrowDown": {
        const dir = e.key === "ArrowDown" ? -1 : 1;
        if (e.altKey && q) setQ(b, clamp(qAfter(b.q, -dir * 12 * fine), q.info.min, q.info.max));
        else if (gain && usesGain(b.shape)) gain.setPlain(b.gainDb + dir * fine);
        else return;
        break;
      }
      case "Enter":
      case " ":
        toggleOn(b);
        break;
      default:
        return;
    }
    e.preventDefault();
    setSelected(b.index);
  };

  const shown = bands.find((b) => b.index === (dragging ? selected : hover));
  const summary = bands.filter((b) => b.on).length;

  return (
    <TypedFrame type="eq-curve" size={size} label={label}>
      <div
        ref={wrap}
        className={clsx("eth-eq", dragging && "eth-eq--dragging", hover !== null && "eth-eq--over-handle")}
        onContextMenu={onContextMenu}
        onPointerMove={(e) => {
          if (grab.current) return;
          const pt = localPoint(e);
          const hit = pt ? hitBand(live.current.plot, live.current.bands, pt.x, pt.y, selected) : null;
          if (hit !== hover) setHover(hit);
        }}
        onPointerLeave={() => setHover(null)}
      >
        <Plot
          className="eth-plot--eq"
          label={`EQ curve, ${summary} of ${bands.length} bands on`}
          testId="widget-eq-curve"
          begin={() => {
            endWheel();
            sender.begin();
          }}
          end={sender.end}
          onDoubleClick={() => {
            const p = lastPress.current;
            if (!p || performance.now() - p.at > DOUBLE_MS) return;
            const b = live.current.bands.find((x) => x.index === p.index);
            if (b) toggleOn(b);
            lastPress.current = null;
          }}
          drag={{
            start: (x, y, e) => {
              const { plot, bands: bs, crossovers: xs } = live.current;
              const hit = hitBand(plot, bs, x, y, selected);
              if (hit !== null) {
                const b = bs.find((v) => v.index === hit)!;
                lastPress.current = { index: hit, at: performance.now() };
                setSelected(hit);
                setDragging(true);
                grab.current = { type: "band", index: hit, mode: e.altKey ? "q" : "move", x, y, fx: freqToX(b.freq), gain: b.gainDb, q: b.q };
                return;
              }
              const split = xs.find((c) => Math.abs(xOf(plot, c.plain) - x) <= HIT / 2);
              if (split) {
                setDragging(true);
                grab.current = { type: "crossover", param: split.info.id };
                return;
              }
              lastPress.current = null;
              return false;
            },
            move: (x, y, e) => {
              const g = grab.current;
              const { plot, bands: bs, crossovers: xs } = live.current;
              if (!g) return;
              if (g.type === "crossover") {
                xs.find((c) => c.info.id === g.param)?.setPlain(xToFreq(clamp(x / Math.max(1, plot.w), 0, 1)));
                return;
              }
              const b = bs.find((v) => v.index === g.index);
              if (!b) return;
              // Incremental so Shift (fine) can be pressed and released mid-drag.
              const fine = e.shiftKey ? 0.1 : 1;
              const dx = (x - g.x) * fine;
              const dy = (y - g.y) * fine;
              g.x = x;
              g.y = y;
              const { freq, gain, q } = b.bindings;
              if (g.mode === "q") {
                if (!q) return;
                g.q = clamp(qAfter(g.q, dy), q.info.min, q.info.max);
                setQ(b, g.q);
                return;
              }
              g.fx = clamp(g.fx + dx / Math.max(1, plot.w), 0, 1);
              freq.setPlain(xToFreq(g.fx));
              if (gain && usesGain(b.shape)) {
                g.gain = clamp(g.gain - dy / pxPerDb(plot), gain.info.min, gain.info.max);
                gain.setPlain(g.gain);
              }
            },
            end: () => {
              grab.current = null;
              setDragging(false);
            },
          }}
        >
          {(box) => {
            live.current.plot = box;
            return (
              <EqGraphics
                box={box}
                bands={bands}
                curves={curves}
                sum={sum}
                selected={selected}
                hover={hover}
                crossovers={crossovers}
                spectra={w.spectrum === "None" ? null : spectra}
                showPre={w.spectrum === "PrePost"}
                onKeyDown={onKeyDown}
                onFocus={setSelected}
              />
            );
          }}
        </Plot>
        {shown && (
          <div className={clsx("eth-eq__readout", `eth-eq__band--${shown.index % 8}`)} data-testid="eq-readout" data-band={shown.index}>
            {readout(shown)}
          </div>
        )}
      </div>
    </TypedFrame>
  );
}

interface GraphicsProps {
  box: PlotBox;
  bands: ReadonlyArray<ResolvedBand>;
  curves: ReadonlyArray<ReadonlyArray<number>>;
  sum: ReadonlyArray<number>;
  selected: number | null;
  hover: number | null;
  crossovers: ReadonlyArray<ParamBinding>;
  spectra: ReturnType<typeof useEqSpectra> | null;
  showPre: boolean;
  onKeyDown: (b: ResolvedBand) => (e: KeyboardEvent) => void;
  onFocus: (index: number) => void;
}

/** SVG points of a dB curve sampled at `FREQS`. */
function curvePoints(box: PlotBox, db: ReadonlyArray<number>): string {
  const n = db.length;
  let s = "";
  for (let i = 0; i < n; i++) s += `${((i / (n - 1)) * box.w).toFixed(1)},${yOf(box, db[i]!).toFixed(1)} `;
  return s;
}

/** SVG points of a spectrum frame (log-spaced bins from `min_hz` to `max_hz`). */
function spectrumPoints(box: PlotBox, f: { min_hz: number; max_hz: number; bins_db: ReadonlyArray<number> }): string {
  const n = f.bins_db.length;
  if (n < 2 || !(f.min_hz > 0) || !(f.max_hz > f.min_hz)) return "";
  let s = "";
  for (let i = 0; i < n; i++) {
    const hz = f.min_hz * Math.pow(f.max_hz / f.min_hz, (i + 0.5) / n);
    s += `${xOf(box, hz).toFixed(1)},${spectrumY(box, f.bins_db[i]!).toFixed(1)} `;
  }
  return s;
}

function EqGraphics({ box, bands, curves, sum, selected, hover, crossovers, spectra, showPre, onKeyDown, onFocus }: GraphicsProps) {
  const zero = yOf(box, 0);
  const sumPts = curvePoints(box, sum);
  const focus = bands.find((b) => b.index === (hover ?? selected));
  const focusCurve = focus && focus.on ? curves[bands.indexOf(focus)] : undefined;
  const pre = spectra && showPre && spectra.pre ? spectrumPoints(box, spectra.pre) : "";
  const post = spectra?.post ? spectrumPoints(box, spectra.post) : "";
  return (
    <>
      <g className="eth-eq__grid" aria-hidden="true">
        {FREQ_TICKS.map((f) => (
          <line key={`f${f}`} className={clsx(FREQ_LABELS.includes(f) && "eth-eq__grid--major")} x1={xOf(box, f)} x2={xOf(box, f)} y1={0} y2={box.h} />
        ))}
        {DB_TICKS.map((d) => (
          <line key={`d${d}`} className={clsx(d === 0 && "eth-eq__grid--zero")} x1={0} x2={box.w} y1={yOf(box, d)} y2={yOf(box, d)} />
        ))}
      </g>
      {pre && <polygon className="eth-eq__spectrum-pre" points={`${pre} ${box.w},${box.h} 0,${box.h}`} data-testid="eq-spectrum-pre" />}
      {post && (
        <>
          <polygon className="eth-eq__spectrum-post-fill" points={`${post} ${box.w},${box.h} 0,${box.h}`} />
          <polyline className="eth-eq__spectrum-post" points={post} data-testid="eq-spectrum-post" />
        </>
      )}
      <g className="eth-eq__labels" aria-hidden="true">
        {FREQ_LABELS.map((f) => (
          <text key={`f${f}`} className="eth-eq__label eth-eq__label--freq" x={xOf(box, f)} y={box.h - 2}>
            {freqLabel(f)}
          </text>
        ))}
        {[DB_RANGE / 2, 0, -DB_RANGE / 2].map((d) => (
          <text key={`d${d}`} className="eth-eq__label eth-eq__label--db" x={4} y={yOf(box, d)}>
            {d > 0 ? `+${d}` : d}
          </text>
        ))}
      </g>
      {focusCurve && (
        <polygon
          className={clsx("eth-eq__band-fill", `eth-eq__band--${focus!.index % 8}`)}
          points={`0,${zero} ${curvePoints(box, focusCurve)} ${box.w},${zero}`}
          data-testid="eq-band-curve"
        />
      )}
      <polyline className="eth-eq__sum" points={sumPts} data-testid="eq-sum" />
      {crossovers.map((c) => (
        <line key={c.info.id} className="eth-eq__crossover" x1={xOf(box, c.plain)} x2={xOf(box, c.plain)} y1={0} y2={box.h} data-handle={c.info.name} />
      ))}
      {bands.map((b) => {
        const { x, y } = handleAt(box, b);
        const active = b.index === selected;
        return (
          <g
            key={b.index}
            className={clsx("eth-eq__handle", `eth-eq__band--${b.index % 8}`, !b.on && "eth-eq__handle--off", active && "eth-eq__handle--selected")}
            transform={`translate(${x.toFixed(1)} ${y.toFixed(1)})`}
            tabIndex={0}
            role="button"
            aria-label={readout(b)}
            aria-pressed={b.on}
            data-band={b.index}
            data-on={b.on ? "true" : "false"}
            onKeyDown={onKeyDown(b)}
            onFocus={() => onFocus(b.index)}
          >
            <circle className="eth-eq__handle-dot" r={9} />
            <text className="eth-eq__handle-num">{b.index + 1}</text>
          </g>
        );
      })}
    </>
  );
}
