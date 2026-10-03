/**
 * Data widgets of the catalog: SampleWaveform (device kind data), Spectrum, Tuner,
 * Meter (`AnalysisData` through the `fx-analysis` seam), RackChains and Macros (racks), and
 * the EqCurve placeholder (the interactive curve is `graphical-eq`'s, in `layout/eq/`). The
 * ZoneMap is `multisampler`'s, in `layout/zones/`.
 */

import clsx from "clsx";
import { useEffect, useRef, useState } from "react";
import type { AnalysisData, MediaRef, ParamInfo, PeakData, Widget, WidgetSize } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, useTransport } from "@/transport";
import { SampleSlot } from "../../SampleSlot";
import { useLayoutContext, useParam, type ParamBinding } from "../context";
import { freqToX, samplePath } from "../curves";
import { GridLines, Plot } from "../plot";
import { useDeviceAnalysis } from "../seams";
import { WidgetView } from "../Widget";
import { Missing, TypedFrame, type TypedProps } from "./typed";

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);
const HIT = 14;

// ---- Sample waveform -------------------------------------------------------------------

/** Peaks fetched for the overview. */
const OVERVIEW_PEAKS = 1024;

/** Whole-sample min/max peaks of `media`, refetched when the engine reports new peaks. */
function useOverviewPeaks(media: MediaRef | undefined): PeakData | null {
  const transport = useTransport();
  const [peaks, setPeaks] = useState<{ media: string; data: PeakData } | null>(null);
  const [version, setVersion] = useState(0);
  const id = media?.id;
  const frames = media?.frames ?? 0;
  useEffect(() => {
    if (!id) return;
    return transport.onEvent((e) => {
      if (e.type === "Media" && e.event.type === "PeaksReady" && e.event.media === id) setVersion((v) => v + 1);
    });
  }, [transport, id]);
  useEffect(() => {
    if (!id || frames <= 0) return;
    let active = true;
    let level = 16;
    while (level * OVERVIEW_PEAKS < frames) level *= 2;
    transport
      .send(cmd("Media", { type: "GetPeaks", request: { media: id, samples_per_peak: level, start_frame: 0, frame_count: frames } }))
      .then((reply) => {
        if (active && reply.type === "Peaks") setPeaks({ media: id, data: reply.peaks });
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [transport, id, frames, version]);
  return peaks && peaks.media === id ? peaks.data : null;
}

/** SVG path of min/max columns (`cols` wide, `h` tall) for `peaks` over `frames`. */
function peaksPath(peaks: PeakData, frames: number, cols: number, h: number): string {
  const n = peaks.max[0]?.length ?? 0;
  const spp = peaks.samples_per_peak;
  let d = "";
  for (let x = 0; x < cols; x++) {
    const i0 = Math.floor(((x / cols) * frames) / spp);
    const i1 = Math.max(i0 + 1, Math.floor((((x + 1) / cols) * frames) / spp));
    let lo = 0;
    let hi = 0;
    for (let ch = 0; ch < peaks.max.length; ch++) {
      for (let i = i0; i < Math.min(i1, n); i++) {
        hi = Math.max(hi, peaks.max[ch]![i]!);
        lo = Math.min(lo, peaks.min[ch]![i]!);
      }
    }
    const y0 = ((1 - hi) / 2) * h;
    const y1 = Math.max(y0 + 1, ((1 - lo) / 2) * h);
    d += `M${x + 0.5} ${y0.toFixed(1)}V${y1.toFixed(1)}`;
  }
  return d;
}

/** The media a device plays: the sampler's sample, or the first zone of a multisampler. */
function useDeviceMedia(): MediaRef | undefined {
  const { device } = useLayoutContext();
  const kind = device.kind;
  const id =
    kind.type === "Builtin" && kind.device.type === "Sampler"
      ? kind.device.sample
      : kind.type === "Builtin" && kind.device.type === "MultiSampler"
        ? (kind.device.zones[0]?.media ?? null)
        : null;
  return useProjectStore((s) => (id ? s.project?.media[id] : undefined));
}

export function SampleWaveformWidget({ widget: w, size, label }: TypedProps<"SampleWaveform">) {
  const { device, sender } = useLayoutContext();
  const start = useParam(w.start);
  const end = useParam(w.end);
  const media = useDeviceMedia();
  const peaks = useOverviewPeaks(media);
  const grab = useRef<ParamBinding["info"]["id"] | null>(null);
  const handles = [start, end].filter((b): b is ParamBinding => b !== null);
  return (
    <TypedFrame type="sample-waveform" size={size} label={label} controls={[w.start, w.end]}>
      <SampleSlot device={device} />
      <Plot
        className="eth-plot--waveform"
        label={media ? `Waveform of ${media.name}` : "No sample"}
        testId="widget-sample-waveform"
        begin={sender.begin}
        end={sender.end}
        drag={
          handles.length
            ? {
                start: (x, _y, e) => {
                  const wpx = e.currentTarget.getBoundingClientRect().width || 1;
                  let best: ParamBinding | null = null;
                  let dist = HIT;
                  for (const b of handles) {
                    const d = Math.abs(b.normalized * wpx - x);
                    if (d <= dist) {
                      dist = d;
                      best = b;
                    }
                  }
                  if (!best) return false;
                  grab.current = best.info.id;
                },
                move: (x, _y, e) => {
                  const b = handles.find((h) => h.info.id === grab.current);
                  if (b) b.setNormalized(clamp01(x / (e.currentTarget.getBoundingClientRect().width || 1)));
                },
                end: () => {
                  grab.current = null;
                },
              }
            : undefined
        }
      >
        {({ w: wpx, h }) => (
          <>
            <GridLines w={wpx} h={h} ys={[0.5]} />
            {peaks && media && media.frames > 0 && <path className="eth-plot__wave" d={peaksPath(peaks, media.frames, Math.max(1, Math.round(wpx)), h)} />}
            {start && <rect className="eth-plot__dim" x={0} y={0} width={start.normalized * wpx} height={h} />}
            {end && <rect className="eth-plot__dim" x={end.normalized * wpx} y={0} width={Math.max(0, (1 - end.normalized) * wpx)} height={h} />}
            {handles.map((b) => (
              <line key={b.info.id} className="eth-plot__marker" x1={b.normalized * wpx} x2={b.normalized * wpx} y1={0} y2={h} data-handle={b.info.name} />
            ))}
          </>
        )}
      </Plot>
    </TypedFrame>
  );
}

// ---- Analysis --------------------------------------------------------------------------

function frameOf<T extends AnalysisData["type"]>(frames: ReadonlyArray<AnalysisData>, type: T, pick?: (f: Extract<AnalysisData, { type: T }>) => boolean) {
  return frames.find((f): f is Extract<AnalysisData, { type: T }> => f.type === type && (!pick || pick(f as Extract<AnalysisData, { type: T }>)));
}

/** Default dB range of the spectrum plot (devices with a `Range` param set the floor). */
const SPECTRUM_DB: [number, number] = [-96, 0];
/** Fall of the UI-held peak line, dB per received frame (~30 Hz → ~9 dB/s). */
const PEAK_FALL_DB = 0.3;
/** Frequency labels of the spectrum axis. */
const SPECTRUM_HZ: ReadonlyArray<[number, string]> = [
  [100, "100"],
  [1000, "1k"],
  [10000, "10k"],
];

/**
 * The value of the device's display param named `name` with `unit` (generic: any device
 * exposing e.g. a `Range` in dB or a `Peak Hold` toggle), or `undefined`.
 */
function useDisplayParam(name: string, unit: ParamInfo["unit"]): number | undefined {
  const { device, params } = useLayoutContext();
  for (const info of params.values()) {
    if (info.name === name && info.unit === unit) return device.params[info.id] ?? info.default;
  }
  return undefined;
}

/** A max-held copy of `bins` that falls by `PEAK_FALL_DB` per new frame (`null` when off). */
function usePeakHold(bins: ReadonlyArray<number> | undefined, on: boolean): ReadonlyArray<number> | null {
  const [held, setHeld] = useState<{ from: ReadonlyArray<number> | undefined; peaks: ReadonlyArray<number> }>({ from: undefined, peaks: [] });
  if (!on || !bins) {
    if (held.from !== undefined) setHeld({ from: undefined, peaks: [] });
    return null;
  }
  if (held.from === bins) return held.peaks;
  // Derived from the previous frame's peaks (state updated during render, React's pattern).
  const peaks = held.peaks.length === bins.length ? bins.map((v, i) => Math.max(v, held.peaks[i]! - PEAK_FALL_DB)) : bins;
  setHeld({ from: bins, peaks });
  return peaks;
}

export function SpectrumWidget({ size, label }: TypedProps<"Spectrum">) {
  const { device } = useLayoutContext();
  const frame = frameOf(useDeviceAnalysis(device.id), "Spectrum", (f) => f.stage !== "Pre");
  const floor = useDisplayParam("Range", "Decibels");
  const peakHold = (useDisplayParam("Peak Hold", "Toggle") ?? 0) >= 0.5;
  const peaks = usePeakHold(frame?.bins_db, peakHold);
  const [lo, hi] = [Math.min(floor ?? SPECTRUM_DB[0], SPECTRUM_DB[1] - 6), SPECTRUM_DB[1]];
  return (
    <TypedFrame type="spectrum" size={size} label={label}>
      <Plot className="eth-plot--spectrum" label={`Spectrum, ${lo.toFixed(0)} to ${hi} dB`} testId="widget-spectrum">
        {({ w: wpx, h }) => {
          const yOf = (db: number) => h - clamp01((db - lo) / (hi - lo)) * h;
          const pathOf = (bins: ReadonlyArray<number>) => {
            const n = bins.length;
            if (!frame || n < 2) return "";
            return bins
              .map((db, i) => {
                const f = frame.min_hz * Math.pow(frame.max_hz / frame.min_hz, i / (n - 1));
                return `${(freqToX(f) * wpx).toFixed(1)},${yOf(db).toFixed(1)}`;
              })
              .join(" ");
          };
          const path = pathOf(frame?.bins_db ?? []);
          const peakPath = peaks ? pathOf(peaks) : "";
          // dB grid every quarter of the range, labelled at the left edge.
          const dbTicks = [0.25, 0.5, 0.75].map((f) => hi - f * (hi - lo));
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.25, 0.5, 0.75]} xs={SPECTRUM_HZ.map(([f]) => freqToX(f))} />
              {dbTicks.map((db) => (
                <text key={`db${db}`} className="eth-plot__empty" x={0} y={yOf(db)} style={{ textAnchor: "start" }} data-axis="db">
                  {db.toFixed(0)}
                </text>
              ))}
              {SPECTRUM_HZ.map(([f, text]) => (
                <text key={`hz${f}`} className="eth-plot__empty" x={freqToX(f) * wpx} y={h} style={{ dominantBaseline: "auto" }} data-axis="hz">
                  {text}
                </text>
              ))}
              {path ? (
                <>
                  <polygon className="eth-plot__fill" points={`0,${h} ${path} ${wpx},${h}`} />
                  <polyline className="eth-plot__line" points={path} />
                  {peakPath && <polyline className="eth-plot__ref" fill="none" points={peakPath} data-testid="spectrum-peak" />}
                </>
              ) : (
                <text className="eth-plot__empty" x={wpx / 2} y={h / 2}>
                  No signal
                </text>
              )}
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

const NOTE_NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];

function noteName(note: number): string {
  return `${NOTE_NAMES[((note % 12) + 12) % 12]}${Math.floor(note / 12) - 1}`;
}

export function TunerWidget({ size, label }: TypedProps<"Tuner">) {
  const { device } = useLayoutContext();
  const frame = frameOf(useDeviceAnalysis(device.id), "Tuner");
  const has = !!frame && frame.note !== null && frame.hz !== null;
  const cents = has ? frame.cents : 0;
  // Whole cents for the readout (no "-0 ct").
  const shown = Math.round(cents) || 0;
  const inTune = has && Math.abs(cents) <= 5;
  return (
    <TypedFrame type="tuner" size={size} label={label}>
      <div className={clsx("eth-tuner", inTune && "eth-tuner--in-tune", !has && "eth-tuner--idle")} data-testid="widget-tuner">
        <span className="eth-tuner__note">{has ? noteName(frame.note!) : "–"}</span>
        <span className="eth-tuner__scale" aria-hidden="true">
          <span className="eth-tuner__needle" style={{ left: `${50 + cents}%` }} />
        </span>
        <span className="eth-tuner__readout">{has ? `${shown > 0 ? "+" : ""}${shown} ct · ${frame.hz!.toFixed(1)} Hz` : "No pitch"}</span>
      </div>
    </TypedFrame>
  );
}

export function MeterWidget({ widget: w, size, label }: TypedProps<"Meter">) {
  const { device } = useLayoutContext();
  const frame = frameOf(useDeviceAnalysis(device.id), "Levels");
  const v = frame?.values[w.index];
  const lo = Math.min(w.min_db, w.max_db);
  const hi = Math.max(w.min_db, w.max_db);
  // Fill from the end of the range nearest 0 dB (gain reduction meters grow downwards).
  const fromTop = Math.abs(w.max_db) < Math.abs(w.min_db) && w.max_db <= 0 && hi === 0;
  const f = v === undefined ? 0 : clamp01((v - lo) / (hi - lo || 1));
  const fill = v === undefined ? 0 : fromTop ? 1 - f : f;
  const text = v === undefined ? "–" : `${v.toFixed(1)} dB`;
  return (
    <div className={clsx("eth-widget", "eth-widget--meter", `eth-widget--${size.toLowerCase()}`)} data-widget="Meter">
      <div
        className={clsx("eth-level", fromTop && "eth-level--down")}
        role="meter"
        aria-label={label ?? "Level"}
        aria-valuemin={lo}
        aria-valuemax={hi}
        aria-valuenow={v ?? lo}
        aria-valuetext={text}
        data-testid="widget-meter"
      >
        <span className="eth-level__fill" style={{ height: `${fill * 100}%` }} />
      </div>
      <span className="eth-param__value">{text}</span>
      {label && <span className="eth-param__name">{label}</span>}
    </div>
  );
}

// ---- Racks -----------------------------------------------------------------------------

export function MacrosWidget({ size, label }: TypedProps<"Macros">) {
  const { params } = useLayoutContext();
  const ids = [0, 1, 2, 3, 4, 5, 6, 7].filter((id) => params.has(id));
  if (ids.length === 0) return <Missing type="Macros" />;
  return (
    <TypedFrame type="macros" size={size} label={label}>
      <div className="eth-widget__grid">
        {ids.map((id) => (
          <WidgetView key={id} widget={{ type: "Knob", param: id }} size={size === "Small" ? "Small" : "Medium"} label={params.get(id)!.name} />
        ))}
      </div>
    </TypedFrame>
  );
}

export function RackChainsWidget({ size, label }: TypedProps<"RackChains">) {
  const { device } = useLayoutContext();
  const chains = useProjectStore((s) => s.project?.rack_chains);
  const list = Object.values(chains ?? {})
    .filter((c) => c.rack === device.id)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
  return (
    <TypedFrame type="rack-chains" size={size} label={label}>
      <ul className="eth-rack-chains" data-testid="widget-rack-chains" aria-label="Chains">
        {list.map((c) => (
          <li key={c.id} className={clsx("eth-rack-chains__row", c.mute && "eth-rack-chains__row--muted")}>
            <span className="eth-rack-chains__name">{c.name}</span>
            <span className="eth-rack-chains__meta">{`${c.volume.toFixed(1)} dB`}</span>
          </li>
        ))}
        {list.length === 0 && <li className="eth-rack-chains__empty">No chains</li>}
      </ul>
    </TypedFrame>
  );
}

// ---- EQ curve (placeholder until graphical-eq) ------------------------------------------

/**
 * Static stand-in for the EQ curve: the band frequencies on the log axis. `graphical-eq`
 * replaces it with the interactive curve (`layout/eq/`), registered in `Widget.tsx`.
 */
export function EqCurvePlaceholder({ widget: w, size, label }: { widget: Extract<Widget, { type: "EqCurve" }>; size: WidgetSize; label: string | null }) {
  const ctx = useLayoutContext();
  const bands = w.bands.flatMap((b) => {
    const info = ctx.params.get(b.freq);
    const on = b.on == null ? 1 : (ctx.device.params[b.on] ?? ctx.params.get(b.on)?.default ?? 1);
    return info ? [{ f: ctx.device.params[info.id] ?? info.default, on: on >= 0.5, id: info.id }] : [];
  });
  return (
    <TypedFrame type="eq-curve" size={size} label={label}>
      <Plot className="eth-plot--eq" label="EQ bands" testId="widget-eq-curve">
        {({ w: wpx, h }) => (
          <>
            <GridLines w={wpx} h={h} ys={[0.5]} xs={[freqToX(100), freqToX(1000), freqToX(10000)]} />
            <polyline className="eth-plot__line" points={samplePath(2, wpx, () => h / 2)} />
            {bands.map((b) => (
              <circle key={b.id} className={clsx("eth-plot__handle", !b.on && "eth-plot__handle--off")} cx={freqToX(b.f) * wpx} cy={h / 2} r={4} />
            ))}
          </>
        )}
      </Plot>
    </TypedFrame>
  );
}
