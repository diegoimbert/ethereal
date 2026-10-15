/**
 * Effect-window visualisations (`devices-pass-effects`, catalog widgets appended to
 * `ether_protocol::layout::Widget`): DynamicsCurve (compressor / gate / limiter transfer
 * curve with the live detector level), DelayTaps (echoes), ReverbDecay (tail) and
 * VelocityCurve (MIDI velocity mapping). Each is a plot over the controls of the params it
 * binds, like the other typed widgets, so a layout never repeats them.
 */

import clsx from "clsx";
import type { AnalysisData, ParamId } from "@/generated";
import { useProjectStore } from "@/state";
import { labelIndex } from "@/domain/paramScale";
import { useLayoutContext, useParam, type ParamBinding } from "../context";
import { delayTaps, divisionBeats, dynamicsModeOf, dynamicsOutDb, reverbEnvelope, velocityMap, velocityModeOf, type DynamicsShape } from "../effectCurves";
import { samplePath } from "../curves";
import { GridLines, Plot } from "../plot";
import { useDeviceAnalysis } from "../seams";
import { genericWidget } from "../model";
import { WidgetView } from "../Widget";
import { Missing, TypedFrame, type TypedProps } from "./typed";
import { clamp01 } from "@/kit";
import { sourceParam, useT } from "@/i18n";

/**
 * The controls row under a large effect plot: the device's hero params (threshold, ratio,
 * time, decay...) as large knobs, as they were before the plot existed, then the others at
 * medium size. One grid, so it wraps as one row.
 */
function HeroControls({ hero, rest }: { hero: ReadonlyArray<ParamId | null>; rest: ReadonlyArray<ParamId | null> }) {
  const ctx = useLayoutContext();
  const items = [...hero.map((id) => [id, "Large"] as const), ...rest.map((id) => [id, "Medium"] as const)].flatMap(([id, size]) => {
    const info = id == null ? undefined : ctx.params.get(id);
    return info ? [{ info, size }] : [];
  });
  if (items.length === 0) return null;
  return (
    <div className="eth-widget__controls eth-widget__controls--hero">
      {items.map(({ info, size }) => (
        <WidgetView key={info.id} widget={genericWidget(info)} size={size} label={info.name} />
      ))}
    </div>
  );
}

/** The current choice as the engine names it (English: the curves match on it). */
function currentLabel(b: ParamBinding | null): string | null {
  const labels = b && sourceParam(b.info).labels;
  if (!b || !labels) return null;
  return labels[labelIndex(b.info, b.plain)] ?? null;
}

function levelsValue(frames: ReadonlyArray<AnalysisData>, index: number | null): number | undefined {
  if (index === null) return undefined;
  const f = frames.find((x): x is Extract<AnalysisData, { type: "Levels" }> => x.type === "Levels");
  return f?.values[index];
}

// ---- Dynamics curve --------------------------------------------------------------------

/** dB span of the dynamics plot (both axes): -DB .. 0 dBFS. */
const DYN_DB = 60;

export function DynamicsCurveWidget({ widget: w, size, label }: TypedProps<"DynamicsCurve">) {
  const t = useT();
  const threshold = useParam(w.threshold);
  const ratio = useParam(w.ratio);
  const knee = useParam(w.knee);
  const mode = useParam(w.mode);
  const floor = useParam(w.floor);
  const inputGain = useParam(w.input_gain);
  const { device, sender } = useLayoutContext();
  const level = levelsValue(useDeviceAnalysis(device.id), w.level);
  if (!threshold) return <Missing type="DynamicsCurve" />;
  const shape: DynamicsShape = {
    mode: dynamicsModeOf(currentLabel(mode), ratio !== null),
    threshold: threshold.plain,
    ratio: ratio?.plain ?? 1,
    knee: knee?.plain ?? 0,
    floor: floor?.plain ?? 0,
    inputGain: inputGain?.plain ?? 0,
  };
  // dB → plot coordinates (input on x, output on y).
  const xOf = (db: number, wpx: number) => clamp01((db + DYN_DB) / DYN_DB) * wpx;
  const yOf = (db: number, h: number) => (1 - clamp01((db + DYN_DB) / DYN_DB)) * h;
  const dbOfX = (x: number, wpx: number) => (x / (wpx || 1)) * DYN_DB - DYN_DB;
  const curveLabel = t("devices.widget.dynamics", {
    mode: shape.mode === "limit" ? "limit" : "other",
    threshold: threshold.text,
    has_ratio: ratio ? "yes" : "no",
    ratio: ratio?.text ?? "",
  });
  return (
    <TypedFrame type="dynamics-curve" size={size} label={label}>
      <Plot
        className="eth-plot--square eth-plot--dynamics"
        label={curveLabel}
        testId="widget-dynamics-curve"
        begin={sender.begin}
        end={sender.end}
        onDoubleClick={() => {
          threshold.reset();
          ratio?.reset();
        }}
        drag={{
          start: () => true,
          move: (x, y, e) => {
            const r = e.currentTarget.getBoundingClientRect();
            // x = threshold (input dB; a limiter's ceiling is on the output axis: y).
            if (shape.mode === "limit") threshold.setPlain(-clamp01(y / (r.height || 1)) * DYN_DB);
            else threshold.setPlain(dbOfX(x, r.width));
            // y (compressor / expander) = ratio, from the top (1:1) down (max).
            if (ratio && shape.mode !== "limit" && shape.mode !== "gate") ratio.setNormalized(clamp01(y / (r.height || 1)));
          },
        }}
      >
        {({ w: wpx, h }) => {
          const path = samplePath(96, wpx, (t) => yOf(dynamicsOutDb(shape, t * DYN_DB - DYN_DB), h));
          const tx = shape.mode === "limit" ? xOf(shape.threshold - shape.inputGain, wpx) : xOf(shape.threshold, wpx);
          const dot =
            level !== undefined && level > -DYN_DB ? { x: xOf(level - shape.inputGain, wpx), y: yOf(dynamicsOutDb(shape, level - shape.inputGain), h) } : null;
          return (
            <>
              <GridLines w={wpx} h={h} ys={[1 / 3, 2 / 3]} xs={[1 / 3, 2 / 3]} />
              <line className="eth-plot__ref" x1={0} y1={h} x2={wpx} y2={0} />
              <line className="eth-plot__marker eth-plot__marker--dim" x1={tx} x2={tx} y1={0} y2={h} data-handle="threshold" />
              <polygon className="eth-plot__fill" points={`0,${h} ${path} ${wpx},${h}`} />
              <polyline className="eth-plot__line" points={path} />
              {dot && <circle className="eth-plot__handle eth-plot__level" cx={dot.x} cy={dot.y} r={4} data-testid="dynamics-level" />}
            </>
          );
        }}
      </Plot>
      {size === "Large" ? (
        <HeroControls hero={[w.threshold, w.ratio, w.input_gain]} rest={[w.knee, w.mode, w.floor]} />
      ) : (
        <HeroControls hero={[]} rest={[w.threshold, w.ratio, w.input_gain, w.knee, w.mode, w.floor]} />
      )}
    </TypedFrame>
  );
}

// ---- Delay taps ------------------------------------------------------------------------

/** Seconds shown by the echo plot (at least three repeats). */
const TAPS_MIN_SPAN = 2;

/** Project tempo at beat 0 (synced delay times). */
function useTempo(): number {
  return useProjectStore((s) => {
    const points = s.project ? Object.values(s.project.tempo_points) : [];
    const first = points.reduce<(typeof points)[number] | undefined>((a, p) => (!a || p.time < a.time ? p : a), undefined);
    return first?.bpm ?? 120;
  });
}

export function DelayTapsWidget({ widget: w, size, label }: TypedProps<"DelayTaps">) {
  const t = useT();
  const time = useParam(w.time);
  const feedback = useParam(w.feedback);
  const sync = useParam(w.sync);
  const division = useParam(w.division);
  const pingPong = useParam(w.ping_pong);
  const freeze = useParam(w.freeze);
  const bpm = useTempo();
  const { sender } = useLayoutContext();
  if (!time || !feedback) return <Missing type="DelayTaps" />;
  const synced = !!sync && sync.plain >= 0.5;
  const beats = synced ? divisionBeats(currentLabel(division)) : null;
  const seconds = beats !== null ? (beats * 60) / bpm : time.plain / 1000;
  const frozen = !!freeze && freeze.plain >= 0.5;
  const ping = !!pingPong && pingPong.plain >= 0.5;
  const fb = feedback.info.unit === "Percent" ? feedback.plain / 100 : feedback.normalized;
  const span = Math.max(TAPS_MIN_SPAN, seconds * 4);
  const taps = delayTaps(seconds, fb, frozen, span);
  const timeText = synced && division ? division.text : time.text;
  return (
    <TypedFrame type="delay-taps" size={size} label={label} controls={[w.time, w.sync, w.division, w.feedback, w.ping_pong, w.freeze]}>
      <Plot
        className="eth-plot--taps"
        label={t("devices.widget.delay", { time: timeText, feedback: feedback.text, ping: ping ? "yes" : "no", frozen: frozen ? "yes" : "no" })}
        testId="widget-delay-taps"
        begin={sender.begin}
        end={sender.end}
        drag={{
          start: () => true,
          move: (x, y, e) => {
            const r = e.currentTarget.getBoundingClientRect();
            // x = the first echo's time (free time only), y = feedback.
            if (!synced) time.setPlain((x / (r.width || 1)) * span * 1000);
            feedback.setNormalized(clamp01(1 - y / (r.height || 1)));
          },
        }}
      >
        {({ w: wpx, h }) => {
          const bar = Math.max(2, Math.min(8, wpx / 48));
          const mid = h / 2;
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.5]} xs={[]} />
              <rect className="eth-plot__bar eth-plot__bar--dry" x={0} width={bar} y={ping ? mid - h * 0.45 : h * 0.05} height={ping ? h * 0.9 : h * 0.95} />
              {taps.map((tap, i) => {
                const x = (tap.t / span) * wpx;
                const len = tap.level * (ping ? h * 0.45 : h * 0.95);
                const y = ping ? (tap.side === 0 ? mid - len : mid) : h - len;
                return (
                  <rect
                    key={i}
                    className={clsx("eth-plot__bar", ping && tap.side === 1 && "eth-plot__bar--alt")}
                    x={x - bar / 2}
                    width={bar}
                    y={y}
                    height={Math.max(1, len)}
                    data-tap={i}
                  />
                );
              })}
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

// ---- Reverb decay ----------------------------------------------------------------------

export function ReverbDecayWidget({ widget: w, size, label }: TypedProps<"ReverbDecay">) {
  const t = useT();
  const decay = useParam(w.decay);
  const preDelay = useParam(w.pre_delay);
  const roomSize = useParam(w.size);
  const damping = useParam(w.damping);
  const freeze = useParam(w.freeze);
  const { sender } = useLayoutContext();
  if (!decay) return <Missing type="ReverbDecay" />;
  const frozen = !!freeze && freeze.plain >= 0.5;
  // Seconds, or a percentage of a nominal 4 s tail (convolution: percent of the IR).
  const decayS = decay.info.unit === "Seconds" ? decay.plain : decay.info.unit === "Milliseconds" ? decay.plain / 1000 : decay.normalized * 4;
  const pre = preDelay ? preDelay.plain / 1000 : 0;
  const span = Math.max(1, Math.min(24, (pre + decayS) * 1.25));
  const damp = damping ? damping.normalized : 0;
  const sizeN = roomSize ? roomSize.normalized : 0.5;
  return (
    <TypedFrame type="reverb-decay" size={size} label={label} controls={[w.decay, w.pre_delay, w.size, w.damping, w.freeze]}>
      <Plot
        className="eth-plot--decay"
        label={t("devices.widget.reverb", {
          decay: decay.text,
          has_pre: preDelay ? "yes" : "no",
          pre: preDelay?.text ?? "",
          frozen: frozen ? "yes" : "no",
        })}
        testId="widget-reverb-decay"
        begin={sender.begin}
        end={sender.end}
        onDoubleClick={() => decay.reset()}
        drag={{
          start: () => true,
          move: (x, _y, e) => {
            const r = e.currentTarget.getBoundingClientRect();
            const s = (x / (r.width || 1)) * span - pre;
            if (decay.info.unit === "Seconds") decay.setPlain(Math.max(0, s));
            else if (decay.info.unit === "Milliseconds") decay.setPlain(Math.max(0, s) * 1000);
            else decay.setNormalized(clamp01(s / 4));
          },
        }}
      >
        {({ w: wpx, h }) => {
          const env = (t: number) => reverbEnvelope(t, pre, frozen ? Infinity : decayS);
          // Amplitude on a 60 dB scale so the fall reads as a straight line.
          const yOf = (a: number) => (a <= 0 ? h : h - clamp01(1 + Math.log10(Math.max(a, 1e-4)) / 3) * h * 0.9);
          const path = samplePath(96, wpx, (t) => yOf(env(t * span)));
          // Damping: the high-frequency part of the tail falls faster (a second, steeper line).
          const bright = samplePath(96, wpx, (t) => yOf(env(t * span) ** (1 + damp * 1.5)));
          const px = (pre / span) * wpx;
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.1 + 0.3, 0.1 + 0.6]} xs={[]} />
              <rect className="eth-plot__dim" x={0} y={0} width={px} height={h} data-testid="reverb-predelay" />
              <polygon className="eth-plot__fill" points={`0,${h} ${path} ${wpx},${h}`} style={{ opacity: 0.4 + 0.6 * sizeN }} />
              <polyline className="eth-plot__line" points={path} />
              {damping && <polyline className="eth-plot__line eth-plot__line--dim" points={bright} />}
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}

// ---- Velocity curve --------------------------------------------------------------------

export function VelocityCurveWidget({ widget: w, size, label }: TypedProps<"VelocityCurve">) {
  const t = useT();
  const drive = useParam(w.drive);
  const compand = useParam(w.compand);
  const inLow = useParam(w.in_low);
  const inHigh = useParam(w.in_high);
  const outLow = useParam(w.out_low);
  const outHigh = useParam(w.out_high);
  const mode = useParam(w.mode);
  const random = useParam(w.random);
  const { sender } = useLayoutContext();
  if (!drive) return <Missing type="VelocityCurve" />;
  const shape = {
    drive: drive.plain,
    compand: compand?.plain ?? 0,
    inLow: inLow?.plain ?? 1,
    inHigh: inHigh?.plain ?? 127,
    outLow: outLow?.plain ?? 1,
    outHigh: outHigh?.plain ?? 127,
    mode: velocityModeOf(currentLabel(mode)),
  };
  const spread = random?.plain ?? 0;
  const ids: ReadonlyArray<ParamId | null> = [w.mode, w.drive, w.compand, w.random];
  return (
    <TypedFrame type="velocity-curve" size={size} label={label} controls={ids}>
      <Plot
        className="eth-plot--square eth-plot--velocity"
        label={t("devices.widget.velocity", { drive: drive.text, has_compand: compand ? "yes" : "no", compand: compand?.text ?? "" })}
        testId="widget-velocity-curve"
        begin={sender.begin}
        end={sender.end}
        onDoubleClick={() => {
          drive.reset();
          compand?.reset();
        }}
        drag={{
          start: () => true,
          move: (_x, y, e) => {
            const r = e.currentTarget.getBoundingClientRect();
            drive.setNormalized(clamp01(1 - y / (r.height || 1)));
          },
        }}
      >
        {({ w: wpx, h }) => {
          const xOf = (v: number) => ((v - 1) / 126) * wpx;
          const yOf = (v: number) => h - ((Math.max(1, Math.min(127, v)) - 1) / 126) * h;
          const pts: string[] = [];
          const segments: string[][] = [];
          for (let i = 0; i <= 126; i++) {
            const v = 1 + i;
            const out = velocityMap(shape, v);
            if (out === null) {
              if (pts.length) segments.push(pts.splice(0));
              continue;
            }
            pts.push(`${xOf(v).toFixed(1)},${yOf(out).toFixed(1)}`);
          }
          if (pts.length) segments.push(pts.splice(0));
          const lo = Math.min(shape.inLow, shape.inHigh);
          const hi = Math.max(shape.inLow, shape.inHigh);
          return (
            <>
              <GridLines w={wpx} h={h} ys={[0.5]} xs={[0.5]} />
              <line className="eth-plot__ref" x1={0} y1={h} x2={wpx} y2={0} />
              {shape.mode !== "fixed" && (
                <>
                  <rect className="eth-plot__dim" x={0} y={0} width={xOf(lo)} height={h} />
                  <rect className="eth-plot__dim" x={xOf(hi)} y={0} width={Math.max(0, wpx - xOf(hi))} height={h} />
                </>
              )}
              {spread > 0 &&
                segments.map((seg, i) => (
                  <polyline key={`r${i}`} className="eth-plot__spread" points={seg.join(" ")} style={{ strokeWidth: Math.max(1, (spread / 126) * h * 2) }} />
                ))}
              {segments.map((seg, i) => (
                <polyline key={i} className="eth-plot__line" points={seg.join(" ")} />
              ))}
            </>
          );
        }}
      </Plot>
    </TypedFrame>
  );
}
