/**
 * Data widgets of the catalog: SampleWaveform, ZoneMap (device kind data), Spectrum, Tuner,
 * Meter (`AnalysisData` through the `fx-analysis` seam), RackChains and Macros (racks), and
 * the EqCurve placeholder (the interactive curve is `graphical-eq`'s, in `layout/eq/`).
 */

import clsx from "clsx";
import { useEffect, useRef, useState } from "react";
import type { AnalysisData, MediaRef, PeakData, Widget, WidgetSize } from "@/generated";
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

// ---- Zone map --------------------------------------------------------------------------

export function ZoneMapWidget({ size, label }: TypedProps<"ZoneMap">) {
  const { device } = useLayoutContext();
  const kind = device.kind;
  const zones = kind.type === "Builtin" && kind.device.type === "MultiSampler" ? kind.device.zones : [];
  return (
    <TypedFrame type="zone-map" size={size} label={label}>
      <Plot className="eth-plot--zones" label={`${zones.length} zones`} testId="widget-zone-map">
        {({ w: wpx, h }) => (
          <>
            <GridLines w={wpx} h={h} ys={[0.5]} xs={[1, 2, 3, 4, 5, 6, 7, 8, 9].map((o) => (o * 12) / 128)} />
            {zones.map((z, i) => {
              const x0 = (z.keys.lo / 128) * wpx;
              const x1 = ((z.keys.hi + 1) / 128) * wpx;
              const y0 = (1 - (z.velocities.hi + 1) / 128) * h;
              const y1 = (1 - Math.max(0, z.velocities.lo - 1) / 128) * h;
              return (
                <rect key={i} className="eth-plot__zone" x={x0} y={y0} width={Math.max(1, x1 - x0)} height={Math.max(1, y1 - y0)}>
                  <title>{`Keys ${z.keys.lo}–${z.keys.hi}, velocity ${z.velocities.lo}–${z.velocities.hi}`}</title>
                </rect>
              );
            })}
            {zones.length === 0 && (
              <text className="eth-plot__empty" x={wpx / 2} y={h / 2}>
                No zones
              </text>
            )}
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

/** dB range of the spectrum plot. */
const SPECTRUM_DB: [number, number] = [-96, 0];

export function SpectrumWidget({ size, label }: TypedProps<"Spectrum">) {
  const { device } = useLayoutContext();
  const frame = frameOf(useDeviceAnalysis(device.id), "Spectrum", (f) => f.stage !== "Pre");
  return (
    <TypedFrame type="spectrum" size={size} label={label}>
      <Plot className="eth-plot--spectrum" label="Spectrum" testId="widget-spectrum">
        {({ w: wpx, h }) => {
          const [lo, hi] = SPECTRUM_DB;
          const bins = frame?.bins_db ?? [];
          const n = bins.length;
          const yOf = (db: number) => h - clamp01((db - lo) / (hi - lo)) * h;
          const path =
            frame && n > 1
              ? bins
                  .map((db, i) => {
                    const f = frame.min_hz * Math.pow(frame.max_hz / frame.min_hz, i / (n - 1));
                    return `${(freqToX(f) * wpx).toFixed(1)},${yOf(db).toFixed(1)}`;
                  })
                  .join(" ")
              : "";
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.25, 0.5, 0.75]} xs={[freqToX(100), freqToX(1000), freqToX(10000)]} />
              {path ? (
                <>
                  <polygon className="eth-plot__fill" points={`0,${h} ${path} ${wpx},${h}`} />
                  <polyline className="eth-plot__line" points={path} />
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
  const inTune = has && Math.abs(cents) <= 5;
  return (
    <TypedFrame type="tuner" size={size} label={label}>
      <div className={clsx("eth-tuner", inTune && "eth-tuner--in-tune", !has && "eth-tuner--idle")} data-testid="widget-tuner">
        <span className="eth-tuner__note">{has ? noteName(frame.note!) : "–"}</span>
        <span className="eth-tuner__scale" aria-hidden="true">
          <span className="eth-tuner__needle" style={{ left: `${50 + cents}%` }} />
        </span>
        <span className="eth-tuner__readout">{has ? `${cents > 0 ? "+" : ""}${cents.toFixed(0)} ct · ${frame.hz!.toFixed(1)} Hz` : "No pitch"}</span>
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
  const fill = fromTop ? 1 - f : f;
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
