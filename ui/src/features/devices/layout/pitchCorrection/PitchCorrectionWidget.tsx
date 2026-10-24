/**
 * The pitch correction's panel (`Widget::PitchCorrection`): the last seconds of the
 * detected pitch (dim) and the pitch played (accent) on a keyboard grid, the notes of the
 * key / scale shaded, and the readouts (target note, detected note and cents, the
 * correction). Reads the device's `AnalysisData::Levels` frame (layout in
 * `ether_devices::pitch_correction::Frame`).
 */

import clsx from "clsx";
import type { AnalysisData } from "@/generated";
import { isNoteInScale, SCALE_KINDS } from "@/domain/scales";
import { useT } from "@/i18n";
import { useLayoutContext, useParam } from "../context";
import { Plot } from "../plot";
import { useDeviceAnalysis } from "../seams";
import { TypedFrame, type TypedProps } from "../widgets/typed";
import { decodePitchCorrection, pitchName, pitchRuns } from "./frame";
import "./pitchCorrection.css";

/** Notes shown around the centre. */
const SPAN = 6;
/** Seconds of history (200 points of 20 ms). */
const SECONDS = 4;

function Readout({ name, value, tone }: { name: string; value: string; tone?: "ok" | "warn" }) {
  return (
    <div className={clsx("eth-pitch-correction__readout", tone && `eth-pitch-correction__readout--${tone}`)} data-readout={name}>
      <span className="eth-pitch-correction__value">{value}</span>
      <span className="eth-pitch-correction__name">{name}</span>
    </div>
  );
}

export function PitchCorrectionWidget({ widget: w, size, label }: TypedProps<"PitchCorrection">) {
  const t = useT();
  const { device } = useLayoutContext();
  const frames = useDeviceAnalysis(device.id);
  const frame = frames.find((f): f is Extract<AnalysisData, { type: "Levels" }> => f.type === "Levels");
  const reading = frame ? decodePitchCorrection(frame.values) : null;
  const key = Math.round(useParam(w.key)?.plain ?? 0);
  const scale = SCALE_KINDS[Math.round(useParam(w.scale)?.plain ?? 0)] ?? "Chromatic";
  // The grid follows the voice: centred on the last voiced pitch played (middle C before).
  const recent = [...(reading?.outputHistory ?? [])].reverse().find((v) => v >= 0);
  const centre = recent === undefined ? 60 : Math.round(recent);
  const lo = centre - SPAN;
  const hi = centre + SPAN;
  const voiced = reading !== null && reading.detected >= 0;
  const cents = voiced ? Math.round(100 * (reading.detected - Math.round(reading.detected))) : 0;
  const fmtCents = (c: number) => `${c > 0 ? "+" : ""}${c}`;
  return (
    <TypedFrame type="pitch-correction" size={size} label={label}>
      <div className="eth-pitch-correction" data-testid="widget-pitch-correction">
        <div className="eth-pitch-correction__readouts">
          <Readout
            name={t("devices.pitch_correction.target")}
            value={voiced && reading.target >= 0 ? pitchName(reading.target) : "–"}
            tone={voiced ? "ok" : undefined}
          />
          <Readout name={t("devices.pitch_correction.detected")} value={voiced ? `${pitchName(reading.detected)} ${fmtCents(cents)}` : "–"} />
          <Readout
            name={t("devices.pitch_correction.correction")}
            value={voiced ? fmtCents(Math.round(reading.correction)) : "–"}
            tone={voiced && Math.abs(reading.correction) > 50 ? "warn" : undefined}
          />
        </div>
        <Plot className="eth-pitch-correction__trace" label={t("devices.widget.pitch_correction")} testId="pitch-correction-trace">
          {({ w: wpx, h }) => {
            const rows = hi - lo + 1;
            const rowH = h / rows;
            const y = (note: number) => (hi + 0.5 - note) * rowH;
            const n = reading?.detectedHistory.length ?? 0;
            const step = reading?.step ?? 0.02;
            const x = (i: number) => wpx - ((n - 1 - i) * step * wpx) / SECONDS;
            const notes = Array.from({ length: rows }, (_, k) => hi - k);
            return (
              <>
                {notes.map((note) => (
                  <g key={note} data-note={note}>
                    {isNoteInScale(note, key, scale) && (
                      <rect className="eth-plot__band--alt" x={0} y={y(note) - rowH / 2} width={wpx} height={rowH} data-in-scale="" />
                    )}
                    {((note % 12) + 12) % 12 === key && (
                      <text className="eth-plot__empty" x={2} y={y(note)} style={{ textAnchor: "start" }} data-axis="note">
                        {pitchName(note)}
                      </text>
                    )}
                  </g>
                ))}
                {reading &&
                  pitchRuns(reading.detectedHistory, x, y).map((pts, i) => (
                    <polyline key={`d${i}`} className="eth-plot__line--dim" fill="none" points={pts} data-testid="pitch-detected" />
                  ))}
                {reading &&
                  pitchRuns(reading.outputHistory, x, y).map((pts, i) => (
                    <polyline key={`o${i}`} className="eth-plot__line" points={pts} data-testid="pitch-output" />
                  ))}
                {n < 2 && (
                  <text className="eth-plot__empty" x={wpx / 2} y={h / 2}>
                    {t("devices.widget.no_pitch")}
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
