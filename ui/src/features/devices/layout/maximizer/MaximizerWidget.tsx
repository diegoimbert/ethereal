/**
 * The maximizer's panel (`Widget::Maximizer`): the last seconds of input level (filled),
 * output level (line) and gain reduction (hanging from the top), on one dB scale with the
 * ceiling line, and the readouts (gain reduction now and deepest, output true peak).
 * Reads the device's `AnalysisData::Levels` frame (layout in `ether_devices::maximizer::Frame`).
 */

import clsx from "clsx";
import type { AnalysisData } from "@/generated";
import { useT } from "@/i18n";
import { clamp01 } from "@/kit";
import { useLayoutContext, useParam } from "../context";
import { GridLines, Plot } from "../plot";
import { useDeviceAnalysis } from "../seams";
import { TypedFrame, type TypedProps } from "../widgets/typed";
import { decodeMaximizer } from "./frame";
import "./maximizer.css";

/** The plot spans 0 dB (top) to -RANGE dB, grid every GRID dB. */
const RANGE = 36;
const GRID = 6;
/** Seconds of history shown (the device keeps 200 points of 20 ms). */
const SECONDS = 4;
/** Levels below this read as silence ("–"). */
const QUIET = -80;

const fmt = (v: number | undefined) => (v === undefined || v <= QUIET ? "–" : v.toFixed(1));
/** A gain reduction (dB, <= 0) as a positive amount. */
const fmtGr = (v: number | undefined) => (v === undefined ? "–" : Math.abs(v) < 0.05 ? "0.0" : (-v).toFixed(1));

function Readout({ name, value, unit, tone }: { name: string; value: string; unit: string; tone?: "warn" | "danger" }) {
  return (
    <div className={clsx("eth-maximizer__readout", tone && `eth-maximizer__readout--${tone}`)} data-readout={name}>
      <span className="eth-maximizer__value">{value}</span>
      <span className="eth-maximizer__unit">{unit}</span>
      <span className="eth-maximizer__name">{name}</span>
    </div>
  );
}

export function MaximizerWidget({ widget: w, size, label }: TypedProps<"Maximizer">) {
  const t = useT();
  const { device } = useLayoutContext();
  const frames = useDeviceAnalysis(device.id);
  const frame = frames.find((f): f is Extract<AnalysisData, { type: "Levels" }> => f.type === "Levels");
  const reading = frame ? decodeMaximizer(frame.values) : null;
  const ceiling = useParam(w.ceiling)?.plain ?? -1;
  const threshold = useParam(w.threshold)?.plain ?? 0;
  // Over the ceiling (rounding, a parameter move): flagged.
  const over = reading !== null && reading.outputMax > ceiling + 0.05;
  return (
    <TypedFrame type="maximizer" size={size} label={label}>
      <div className="eth-maximizer" data-testid="widget-maximizer">
        <div className="eth-maximizer__readouts">
          <Readout
            name={t("devices.maximizer.reduction")}
            value={fmtGr(reading?.gainReduction)}
            unit="db"
            tone={reading && reading.gainReduction < -0.05 ? "warn" : undefined}
          />
          <Readout name={t("devices.maximizer.deepest")} value={fmtGr(reading?.gainReductionMax)} unit="db" />
          <Readout name={t("devices.maximizer.output_peak")} value={fmt(reading?.outputMax)} unit="dbtp" tone={over ? "danger" : undefined} />
          <Readout name={t("devices.maximizer.boost")} value={(-threshold).toFixed(1)} unit="db" />
        </div>
        <Plot className="eth-maximizer__history" label={t("devices.widget.maximizer")} testId="maximizer-history">
          {({ w: wpx, h }) => {
            const yOf = (db: number) => clamp01(-db / RANGE) * h;
            const n = reading?.inputHistory.length ?? 0;
            const step = reading?.step ?? 0.02;
            const xOf = (i: number) => wpx - ((n - 1 - i) * step * wpx) / SECONDS;
            const pts = (vals: ReadonlyArray<number>) => vals.map((v, i) => `${xOf(i).toFixed(1)},${yOf(v).toFixed(1)}`).join(" ");
            const grid: number[] = [];
            for (let v = -GRID; v > -RANGE; v -= GRID) grid.push(v);
            const x0 = n ? xOf(0).toFixed(1) : wpx;
            return (
              <>
                <GridLines w={wpx} h={h} ys={grid.map((v) => yOf(v) / (h || 1))} />
                {grid.map((v) => (
                  <text key={v} className="eth-plot__empty" x={0} y={yOf(v)} style={{ textAnchor: "start" }} data-axis="db">
                    {v}
                  </text>
                ))}
                <line className="eth-maximizer__ceiling" x1={0} x2={wpx} y1={yOf(ceiling)} y2={yOf(ceiling)} data-testid="maximizer-ceiling" />
                {n > 1 ? (
                  <>
                    <polygon className="eth-plot__fill" points={`${x0},${h} ${pts(reading!.inputHistory)} ${wpx},${h}`} data-testid="maximizer-input" />
                    <polyline className="eth-plot__line" points={pts(reading!.outputHistory)} data-testid="maximizer-output" />
                    <polygon
                      className="eth-maximizer__reduction"
                      points={`${x0},0 ${pts(reading!.gainReductionHistory)} ${wpx},0`}
                      data-testid="maximizer-reduction"
                    />
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
