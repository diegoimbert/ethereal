/**
 * The adaptive EQ's panel (`Widget::AdaptiveEq`): the curve its band filters make now
 * (accent line, 0 dB in the middle) over the band levels relative to the balanced
 * reference (dim bars: up = sticks out, down = buried), and the auto gain. Reads the
 * device's `AnalysisData::Levels` frame (layout in `ether_devices::adaptive_eq::Frame`).
 */

import type { AnalysisData } from "@/generated";
import { useT } from "@/i18n";
import { clamp01 } from "@/kit";
import { useLayoutContext } from "../context";
import { GridLines, Plot } from "../plot";
import { useDeviceAnalysis } from "../seams";
import { TypedFrame, type TypedProps } from "../widgets/typed";
import { decodeAdaptiveEq } from "./frame";
import "./adaptiveEq.css";

/** The plot spans ±RANGE dB; band levels are drawn at half scale. */
const RANGE = 12;
const MIN_HZ = 20;
const MAX_HZ = 20_000;

export function AdaptiveEqWidget({ size, label }: TypedProps<"AdaptiveEq">) {
  const t = useT();
  const { device } = useLayoutContext();
  const frames = useDeviceAnalysis(device.id);
  const frame = frames.find((f): f is Extract<AnalysisData, { type: "Levels" }> => f.type === "Levels");
  const reading = frame ? decodeAdaptiveEq(frame.values) : null;
  const auto = reading?.autoGain ?? 0;
  return (
    <TypedFrame type="adaptive-eq" size={size} label={label}>
      <div className="eth-adaptive-eq" data-testid="widget-adaptive-eq">
        <div className="eth-adaptive-eq__readout" data-readout="auto gain">
          <span className="eth-adaptive-eq__value">{`${auto > 0 ? "+" : ""}${auto.toFixed(1)}`}</span>
          <span className="eth-adaptive-eq__unit">db</span>
          <span className="eth-adaptive-eq__name">{t("devices.adaptive_eq.auto_gain")}</span>
        </div>
        <Plot className="eth-adaptive-eq__curve" label={t("devices.widget.adaptive_eq")} testId="adaptive-eq-curve">
          {({ w, h }) => {
            const x = (hz: number) => (Math.log(hz / MIN_HZ) / Math.log(MAX_HZ / MIN_HZ)) * w;
            const y = (db: number) => (1 - clamp01((db + RANGE) / (2 * RANGE))) * h;
            const grid = [-6, 0, 6].map((db) => y(db) / (h || 1));
            const half = reading ? Math.max(1, w / reading.bandFreqs.length / 3) : 1;
            return (
              <>
                <GridLines w={w} h={h} ys={grid} />
                {[100, 1000, 10_000].map((hz) => (
                  <text key={hz} className="eth-plot__empty" x={x(hz)} y={h - 2} data-axis="hz">
                    {hz >= 1000 ? `${hz / 1000}k` : hz}
                  </text>
                ))}
                {reading ? (
                  <>
                    {reading.bandFreqs.map((hz, i) => {
                      const lv = Math.max(-RANGE, Math.min(RANGE, (reading.bandLevels[i] ?? 0) / 2));
                      const [a, b] = [y(0), y(lv)];
                      return (
                        <rect
                          key={i}
                          className="eth-plot__bar--alt"
                          x={x(hz) - half}
                          y={Math.min(a, b)}
                          width={2 * half}
                          height={Math.abs(b - a)}
                          data-band={i}
                        />
                      );
                    })}
                    <polyline
                      className="eth-plot__line"
                      points={reading.freqs.map((hz, i) => `${x(hz).toFixed(1)},${y(reading.response[i] ?? 0).toFixed(1)}`).join(" ")}
                      data-testid="adaptive-eq-response"
                    />
                  </>
                ) : (
                  <text className="eth-plot__empty" x={w / 2} y={h / 2}>
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
