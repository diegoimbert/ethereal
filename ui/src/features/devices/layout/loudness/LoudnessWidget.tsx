/**
 * The loudness meter's panel (`Widget::Loudness`): the readouts (momentary, short-term,
 * integrated, loudness range, true peak) above the history of the momentary (thin) and
 * short-term (filled) loudness, with the `target` reference line. Reads the device's
 * `AnalysisData::Levels` frame (layout in `ether_devices::fx_analysis::loudness_device::Frame`).
 */

import clsx from "clsx";
import type { AnalysisData } from "@/generated";
import { useT } from "@/i18n";
import { clamp01 } from "@/kit";
import { useLayoutContext, useParam } from "../context";
import { GridLines, Plot } from "../plot";
import { useDeviceAnalysis } from "../seams";
import { TypedFrame, type TypedProps } from "../widgets/typed";
import { decodeLoudness } from "./frame";
import "./loudness.css";

/** Below this a loudness reads as silence ("–"). */
const QUIET = -70;
/** True peaks above this are flagged (the usual -1 dBTP delivery ceiling). */
const PEAK_WARN = -1;
/** The history spans `target - BELOW` .. `target + ABOVE`, grid every `GRID` LU. */
const BELOW = 27;
const ABOVE = 9;
const GRID = 9;

const fmt = (v: number | undefined) => (v === undefined || v <= QUIET ? "–" : v.toFixed(1));

function Readout({ name, value, unit, tone, big }: { name: string; value: string; unit: string; tone?: "ok" | "warn" | "danger"; big?: boolean }) {
  return (
    <div className={clsx("eth-loudness__readout", big && "eth-loudness__readout--big", tone && `eth-loudness__readout--${tone}`)} data-readout={name}>
      <span className="eth-loudness__value">{value}</span>
      <span className="eth-loudness__unit">{unit}</span>
      <span className="eth-loudness__name">{name}</span>
    </div>
  );
}

export function LoudnessWidget({ widget: w, size, label }: TypedProps<"Loudness">) {
  const t = useT();
  const { device } = useLayoutContext();
  const frames = useDeviceAnalysis(device.id);
  const frame = frames.find((f): f is Extract<AnalysisData, { type: "Levels" }> => f.type === "Levels");
  const reading = frame ? decodeLoudness(frame.values) : null;
  const target = useParam(w.target)?.plain ?? -14;
  const lo = target - BELOW;
  const hi = target + ABOVE;
  const integrated = reading?.integrated;
  const intTone =
    integrated === undefined || integrated <= QUIET ? undefined : Math.abs(integrated - target) <= 1 ? "ok" : integrated > target ? "warn" : undefined;
  const peak = reading?.truePeakMax;
  const peakTone = peak !== undefined && peak > PEAK_WARN ? "danger" : undefined;
  const window = reading?.window ?? 30;
  return (
    <TypedFrame type="loudness" size={size} label={label}>
      <div className="eth-loudness" data-testid="widget-loudness">
        <div className="eth-loudness__readouts">
          <Readout name={t("devices.loudness.integrated")} value={fmt(integrated)} unit="lufs" tone={intTone} big />
          <Readout name={t("devices.loudness.short_term")} value={fmt(reading?.shortTerm)} unit="lufs" />
          <Readout name={t("devices.loudness.momentary")} value={fmt(reading?.momentary)} unit="lufs" />
          <Readout name={t("devices.loudness.range")} value={reading && reading.integrated > QUIET ? reading.range.toFixed(1) : "–"} unit="lu" />
          <Readout name={t("devices.loudness.true_peak")} value={fmt(peak)} unit="dbtp" tone={peakTone} />
        </div>
        <Plot className="eth-loudness__history" label={t("devices.widget.loudness", { seconds: Math.round(window) })} testId="loudness-history">
          {({ w: wpx, h }) => {
            const yOf = (v: number) => (1 - clamp01((v - lo) / (hi - lo))) * h;
            const points = (vals: ReadonlyArray<number>) => {
              const n = vals.length;
              const step = reading?.step ?? 0.1;
              return vals.map((v, i) => `${(wpx - ((n - 1 - i) * step * wpx) / window).toFixed(1)},${yOf(v).toFixed(1)}`).join(" ");
            };
            const grid: number[] = [];
            for (let v = hi - GRID; v > lo; v -= GRID) grid.push(v);
            const short = reading?.shortTermHistory ?? [];
            const mom = reading?.momentaryHistory ?? [];
            const shortPts = points(short);
            const x0 = short.length ? wpx - ((short.length - 1) * (reading?.step ?? 0.1) * wpx) / window : wpx;
            return (
              <>
                <GridLines w={wpx} h={h} ys={grid.map((v) => yOf(v) / (h || 1))} />
                {grid.map((v) => (
                  <text key={v} className="eth-plot__empty" x={0} y={yOf(v)} style={{ textAnchor: "start" }} data-axis="lufs">
                    {v.toFixed(0)}
                  </text>
                ))}
                <line className="eth-loudness__target" x1={0} x2={wpx} y1={yOf(target)} y2={yOf(target)} data-testid="loudness-target" />
                {short.length > 1 ? (
                  <>
                    <polygon className="eth-plot__fill" points={`${x0.toFixed(1)},${h} ${shortPts} ${wpx},${h}`} />
                    <polyline className="eth-plot__line--dim eth-loudness__momentary" fill="none" points={points(mom)} />
                    <polyline className="eth-plot__line" points={shortPts} data-testid="loudness-short-term" />
                  </>
                ) : (
                  <text className="eth-plot__empty" x={wpx / 2} y={h / 2}>
                    {t("devices.widget.no_signal")}
                  </text>
                )}
              </>
            );
          }}
        </Plot>
      </div>
    </TypedFrame>
  );
}
