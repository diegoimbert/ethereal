/**
 * The volume shaper's band strip (`Widget::LfoCurve::bands`): the frequency bands on a log
 * Hz axis (20 Hz..20 kHz), each in its band colour, with a handle on every crossover in use,
 * a band count picker, and one tab per band. Clicking a band (in the strip or its tab)
 * selects the band the curve editor below draws; dragging a handle moves that crossover
 * (one undo gesture per drag), kept between its neighbours.
 */

import clsx from "clsx";
import { useRef } from "react";
import { clamp01 } from "@/kit";
import { useT } from "@/i18n";
import type { ParamBinding } from "../context";
import { freqToX, xToFreq } from "../curves";
import { GridLines, Plot } from "../plot";

/** Pointer distance (px) that grabs a crossover handle. */
const HIT = 10;

export interface BandStripProps {
  /** Bands in use (1..=3). */
  count: number;
  /** The band count choice (plain value = count - 1). */
  countParam: ParamBinding | null;
  /** Every crossover, lowest first (the first `count - 1` are in use). */
  crossovers: ReadonlyArray<ParamBinding>;
  selected: number;
  onSelect(band: number): void;
  begin(): void;
  end(): void;
}

/** Name key of band `b` of `count` bands. */
function bandNameKey(b: number, count: number) {
  if (count <= 1) return "devices.lfo_curve.bands.all" as const;
  if (b === 0) return "devices.lfo_curve.bands.low" as const;
  return b === count - 1 ? ("devices.lfo_curve.bands.high" as const) : ("devices.lfo_curve.bands.mid" as const);
}

/** Crossover positions (0..1) of the ones in use, each at or above the previous one (as the engine plays them). */
function crossoverXs(crossovers: ReadonlyArray<ParamBinding>, count: number): number[] {
  const xs: number[] = [];
  for (const c of crossovers.slice(0, Math.max(0, count - 1))) xs.push(Math.max(freqToX(c.plain), xs[xs.length - 1] ?? 0));
  return xs;
}

/** The band under `x` (0..1). */
function bandAt(xs: ReadonlyArray<number>, x: number): number {
  let b = 0;
  while (b < xs.length && x >= xs[b]!) b++;
  return b;
}

export function BandStrip({ count, countParam, crossovers, selected, onSelect, begin, end }: BandStripProps) {
  const t = useT();
  const grab = useRef<number | null>(null);
  const used = crossovers.slice(0, Math.max(0, count - 1));
  const xs = crossoverXs(crossovers, count);
  const name = (b: number) => t(bandNameKey(b, count));
  const range = (b: number) => {
    if (count <= 1) return t("devices.lfo_curve.bands.range_all");
    if (b === 0) return t("devices.lfo_curve.bands.range_below", { freq: used[0]!.text });
    if (b === count - 1) return t("devices.lfo_curve.bands.range_above", { freq: used[b - 1]!.text });
    return t("devices.lfo_curve.bands.range_between", { from: used[b - 1]!.text, to: used[b]!.text });
  };
  const bands = Array.from({ length: count }, (_, b) => b);
  const setCount = (n: number) => {
    if (!countParam || n === count) return;
    begin();
    countParam.setPlain(n - 1);
    end();
  };
  return (
    <div className="eth-lfo-bands" data-testid="lfo-bands">
      <div className="eth-lfo-bands__head">
        <div className="eth-lfo-bands__count" role="group" aria-label={t("devices.lfo_curve.bands.count")}>
          {[1, 2, 3].map((n) => (
            <button
              key={n}
              type="button"
              className={clsx("eth-lfo-bands__count-option", n === count && "eth-lfo-bands__count-option--on")}
              aria-pressed={n === count}
              aria-label={t("devices.lfo_curve.bands.count_option", { count: n })}
              onClick={() => setCount(n)}
            >
              {n}
            </button>
          ))}
        </div>
        <div className="eth-lfo-bands__tabs" role="tablist" aria-label={t("devices.lfo_curve.bands.tabs")}>
          {bands.map((b) => (
            <button
              key={b}
              type="button"
              role="tab"
              aria-selected={b === selected}
              className={clsx("eth-lfo-bands__tab", `eth-lfo-band--${b}`, b === selected && "eth-lfo-bands__tab--on")}
              title={range(b)}
              data-band={b}
              onClick={() => onSelect(b)}
            >
              <span className="eth-lfo-bands__swatch" aria-hidden="true" />
              <span className="eth-lfo-bands__name">{name(b)}</span>
              <span className="eth-lfo-bands__range">{range(b)}</span>
            </button>
          ))}
        </div>
      </div>
      <Plot
        className="eth-lfo-bands__strip"
        label={t("devices.lfo_curve.bands.strip", { bands: bands.map((b) => `${name(b)} ${range(b)}`).join(", ") })}
        testId="lfo-bands-strip"
        begin={begin}
        end={end}
        drag={{
          start: (x, _y, e) => {
            const w = e.currentTarget.getBoundingClientRect().width || 1;
            let best: number | null = null;
            let dist = HIT;
            for (let i = 0; i < xs.length; i++) {
              const d = Math.abs(xs[i]! * w - x);
              if (d <= dist) {
                dist = d;
                best = i;
              }
            }
            if (best === null) {
              onSelect(bandAt(xs, x / w));
              return false;
            }
            grab.current = best;
          },
          move: (x, _y, e) => {
            const i = grab.current;
            const c = i === null ? undefined : used[i];
            if (i === null || !c) return;
            const w = e.currentTarget.getBoundingClientRect().width || 1;
            // Between its neighbours (the engine keeps the upper one at or above the lower one).
            const lo = i > 0 ? xs[i - 1]! : 0;
            const hi = i < xs.length - 1 ? xs[i + 1]! : 1;
            c.setPlain(xToFreq(Math.min(hi, Math.max(lo, clamp01(x / w)))));
          },
          end: () => {
            grab.current = null;
          },
        }}
      >
        {({ w, h }) => {
          const edges = [0, ...xs, 1];
          return (
            <>
              <title>{t("devices.lfo_curve.bands.hint")}</title>
              {bands.map((b) => (
                <rect
                  key={b}
                  className={clsx("eth-lfo-bands__band", `eth-lfo-band--${b}`, b === selected && "eth-lfo-bands__band--on")}
                  x={edges[b]! * w}
                  y={0}
                  width={Math.max(0, (edges[b + 1]! - edges[b]!) * w)}
                  height={h}
                  data-band={b}
                />
              ))}
              <GridLines w={w} h={h} ys={[]} xs={[freqToX(100), freqToX(1000), freqToX(10000)]} />
              {[100, 1000, 10000].map((f) => (
                <text key={f} className="eth-lfo-bands__tick" x={freqToX(f) * w + 2} y={h - 3}>
                  {f >= 1000 ? `${f / 1000}k` : f}
                </text>
              ))}
              {xs.map((cx, i) => (
                <g key={i} className="eth-lfo-bands__split" data-handle={i}>
                  <line x1={cx * w} x2={cx * w} y1={0} y2={h} />
                  <circle className="eth-plot__handle" cx={cx * w} cy={h / 2} />
                  <text className="eth-lfo-bands__freq" x={cx * w + (cx > 0.85 ? -4 : 4)} y={10} textAnchor={cx > 0.85 ? "end" : "start"}>
                    {used[i]!.text}
                  </text>
                </g>
              ))}
            </>
          );
        }}
      </Plot>
    </div>
  );
}
