/**
 * The Volume LFO's curve editor (`Widget::LfoCurve`): the drawn cycle
 * (`BuiltinDevice::VolumeLfo::curve`) on a grid of the cycle's sixteenth notes, the gain the
 * device plays (depth applied) under it, and a playhead while the transport plays. With
 * `bands` (the multiband volume shaper), the band strip above it (`BandStrip`) selects the
 * band whose cycle it draws (`curve` or `bands[i - 1]`), in that band's colour, at that
 * band's speed, with the band's depth and speed knobs under it.
 *
 * - double-click empty space: add a point there (snapped to the grid);
 * - drag a point: move it between its neighbours (x snapped to the grid);
 * - double-click a point, or Delete on a focused one: delete it (the last one stays);
 * - alt-drag a segment: bend it (`CurveShape::Curve { tension }`, as automation lanes);
 * - alt while dragging: no snap (the app's snap bypass);
 * - right-click a point or a segment: delete, linear, step or smooth curve.
 *
 * Every gesture is one `Device::SetLfoCurve` undo step (the gesture sender); a drag draws
 * from a local draft so it follows the pointer before the document catches up.
 */

import clsx from "clsx";
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent } from "react";
import type { CurveShape, Device, LfoCurve, ParamId, Widget, WidgetSize } from "@/generated";
import { openContextMenu, type ContextMenuEntry } from "@/kit";
import { useT } from "@/i18n";
import { playheadStore } from "@/state/playhead";
import { cmd } from "@/transport";
import { createDoublePress } from "@/timeline/doublePress";
import { bendTension, tensionOf } from "@/features/automation/edit";
import { bindParam, useLayoutContext, useParam, type ParamBinding } from "../context";
import { useBoxSize } from "../useBoxSize";
import { ControlsRow, TypedFrame } from "../widgets/typed";
import { BandStrip } from "./BandStrip";
import {
  DEFAULT_LFO_CURVE,
  addPoint,
  curvePath,
  gridDivisions,
  hitPoint,
  movePoint,
  removePoint,
  sameCurve,
  segmentAt,
  volumeShaperBand,
  segmentRises,
  setSegmentCurve,
  snapX,
  SYNC_RATE_BEATS,
} from "./lfoCurveMath";
import "./lfoCurve.css";

type Spec = Extract<Widget, { type: "LfoCurve" }>;

/** Pointer distance (px) that grabs a point. */
const HIT = 9;

type Grab = { type: "point"; i: number } | { type: "bend"; i: number; t0: number; y0: number; rising: boolean };

/** The curve of band `band` of a Volume LFO (the default pump / flat when absent). */
function curveOf(device: Device, band: number): LfoCurve {
  const k = device.kind;
  if (k.type !== "Builtin" || k.device.type !== "VolumeLfo") return DEFAULT_LFO_CURVE;
  return volumeShaperBand(k.device.curve ?? DEFAULT_LFO_CURVE, k.device.bands, band);
}

type Bands = NonNullable<Spec["bands"]>;

export function LfoCurveWidget({ widget: w, size, label }: { widget: Spec; size: WidgetSize; label: string | null }) {
  return w.bands ? <BandedLfoCurve widget={w} bands={w.bands} size={size} label={label} /> : <SingleLfoCurve widget={w} size={size} label={label} />;
}

/** Cycle timing of the widget's params: synced cycle length (beats) or `null`, the free rate and the phase offset. */
function useTiming(w: Spec) {
  const sync = useParam(w.sync);
  const syncRate = useParam(w.sync_rate);
  const rate = useParam(w.rate);
  const phase = useParam(w.phase);
  const cycleBeats = sync && sync.plain >= 0.5 && syncRate ? (SYNC_RATE_BEATS[Math.round(syncRate.plain)] ?? 1) : null;
  return { cycleBeats, hz: rate?.plain ?? 1, offset: (phase?.plain ?? 0) / 360 };
}

const playedAt = (depth: ParamBinding | null) => {
  const d = depth ? depth.plain / 100 : 1;
  return d < 1 ? (y: number) => 1 - d * (1 - y) : null;
};

function SingleLfoCurve({ widget: w, size, label }: { widget: Spec; size: WidgetSize; label: string | null }) {
  const { sender, device } = useLayoutContext();
  const timing = useTiming(w);
  const depth = useParam(w.depth);
  return (
    <TypedFrame type="lfo-curve" size={size} label={label}>
      <CurveEditor
        curve={curveOf(device, 0)}
        send={(curve) => void sender.send(cmd("Device", { type: "SetLfoCurve", device: device.id, band: 0, curve }))}
        {...timing}
        played={playedAt(depth)}
        testId="widget-lfo-curve"
      />
    </TypedFrame>
  );
}

/** The multiband volume shaper: band strip, the selected band's curve, its depth and speed. */
function BandedLfoCurve({ widget: w, bands: spec, size, label }: { widget: Spec; bands: Bands; size: WidgetSize; label: string | null }) {
  const ctx = useLayoutContext();
  const { sender, device } = ctx;
  const timing = useTiming(w);
  const countParam = useParam(spec.count);
  const count = Math.min(spec.depths.length, Math.max(1, Math.round(countParam?.plain ?? 0) + 1));
  const [picked, setPicked] = useState(0);
  const band = Math.min(picked, count - 1);
  const bind = (id: ParamId | null | undefined) => {
    const info = id == null ? undefined : ctx.params.get(id);
    return info ? bindParam(ctx, info) : null;
  };
  const crossovers = spec.crossovers.flatMap((id) => {
    const b = bind(id);
    return b ? [b] : [];
  });
  const speed = bind(spec.speeds[band]);
  const factor = speed ? (spec.speed_factors[Math.round(speed.plain)] ?? 1) : 1;
  const controls = [spec.depths[band], spec.speeds[band], ...spec.crossovers.slice(0, count - 1)];
  return (
    <TypedFrame type="lfo-curve" size={size} label={label}>
      <BandStrip count={count} countParam={countParam} crossovers={crossovers} selected={band} onSelect={setPicked} begin={sender.begin} end={sender.end} />
      <CurveEditor
        key={band}
        curve={curveOf(device, band)}
        send={(curve) => void sender.send(cmd("Device", { type: "SetLfoCurve", device: device.id, band, curve }))}
        cycleBeats={timing.cycleBeats === null ? null : timing.cycleBeats / factor}
        hz={timing.hz * factor}
        offset={timing.offset}
        played={playedAt(bind(spec.depths[band]))}
        band={count > 1 ? band : undefined}
        testId="widget-lfo-curve"
      />
      <ControlsRow ids={controls} size={size === "Large" ? "Medium" : "Small"} />
    </TypedFrame>
  );
}

export interface CurveEditorProps {
  /** The curve in the document. */
  curve: LfoCurve;
  /** Send an edit (inside the gesture the editor opens). */
  send(curve: LfoCurve): void;
  /** Cycle length in beats (synced: the grid and the song-locked playhead), or `null` (free, at `hz`). */
  cycleBeats: number | null;
  hz: number;
  /** Phase offset (cycles). */
  offset: number;
  /** What the device plays for a drawn value (depth applied), drawn under the curve; `null` = as drawn. */
  played: ((y: number) => number) | null;
  /** Dimmed (the lane is off). */
  off?: boolean;
  /** Drawn in this volume shaper band's colour. */
  band?: number;
  testId: string;
}

/** The drawn-cycle editor shared by the Volume LFO and the shaper's lanes. */
export function CurveEditor({ curve: doc, send, cycleBeats, hz, offset, played, off, band, testId }: CurveEditorProps) {
  const t = useT();
  const { sender } = useLayoutContext();
  const divisions = gridDivisions(cycleBeats);
  const [draft, setDraft] = useState<LfoCurve | null>(null);
  const curve = draft ?? doc;
  const [ref, box] = useBoxSize<HTMLDivElement>();
  const grab = useRef<Grab | null>(null);
  const double = useRef(createDoublePress());
  const live = useRef({ curve, box, divisions, doc });
  useLayoutEffect(() => {
    live.current = { curve, box, divisions, doc };
  });
  // The document caught up with the draft (or changed under it): draw the document.
  useEffect(() => {
    if (!grab.current) setDraft(null);
  }, [doc]);

  /** One edit, one undo step. */
  const commit = (next: LfoCurve) => {
    sender.begin();
    send(next);
    sender.end();
  };

  // Playhead: the engine's phase (synced: the song position; free: since playback started).
  const playhead = useRef<SVGLineElement>(null);
  const timing = useRef({ cycleBeats, hz, offset });
  useLayoutEffect(() => {
    timing.current = { cycleBeats, hz, offset };
  });
  useEffect(() => {
    let started: number | null = null;
    const update = () => {
      const el = playhead.current;
      const f = playheadStore.getPlayhead()?.transport;
      if (!el) return;
      if (!f?.playing) {
        started = null;
        el.style.visibility = "hidden";
        return;
      }
      started ??= f.seconds;
      const { cycleBeats: beats, hz, offset } = timing.current;
      const base = beats !== null ? f.position / beats : (f.seconds - started) * hz;
      const p = base + offset;
      const x = (p - Math.floor(p)) * live.current.box.w;
      el.style.visibility = "visible";
      el.setAttribute("x1", x.toFixed(1));
      el.setAttribute("x2", x.toFixed(1));
    };
    update();
    return playheadStore.subscribePlayhead(update);
  }, []);

  const local = (e: { clientX: number; clientY: number; currentTarget: Element }) => {
    const r = e.currentTarget.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  const onPointerDown = (e: PointerEvent<SVGSVGElement>) => {
    if (e.button !== 0) return;
    const { curve: c, box: b, divisions: div } = live.current;
    const { x, y } = local(e);
    const hit = hitPoint(c.points, x, y, b.w, b.h, HIT);
    const isDouble = double.current(e, hit === null ? "empty" : `point:${hit}`);
    e.preventDefault();
    if (hit !== null && isDouble) {
      commit(removePoint(c, hit));
      return;
    }
    let start: Grab | null = null;
    sender.begin();
    if (hit !== null) start = { type: "point", i: hit };
    else if (isDouble) {
      const added = addPoint(c, snapX(x / b.w, div, e.altKey), 1 - y / b.h);
      if (added) {
        setDraft(added.curve);
        send(added.curve);
        start = { type: "point", i: added.index };
      }
    } else if (e.altKey) {
      const seg = segmentAt(c.points, Math.min(Math.max(x / b.w, 0), 1));
      start = { type: "bend", i: seg.i, t0: tensionOf(c.points[seg.i]!.curve), y0: y, rising: segmentRises(c, seg.i) };
    }
    if (!start) {
      sender.end();
      return;
    }
    grab.current = start;
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<SVGSVGElement>) => {
    const g = grab.current;
    if (!g) return;
    const { curve: c, box: b, divisions: div } = live.current;
    const { x, y } = local(e);
    const next =
      g.type === "point"
        ? movePoint(c, g.i, snapX(x / b.w, div, e.altKey), 1 - y / b.h)
        : setSegmentCurve(c, g.i, { type: "Curve", tension: bendTension(g.t0, g.y0 - y, g.rising) });
    setDraft(next);
    send(next);
  };

  const onPointerUp = () => {
    if (!grab.current) return;
    grab.current = null;
    sender.end();
    // Already in the document: draw it (else the effect above does once it arrives).
    const { curve: c, doc: now } = live.current;
    if (sameCurve(c, now)) setDraft(null);
  };

  const shapeItems = (i: number): ContextMenuEntry[] => {
    const set = (shape: CurveShape) => commit(setSegmentCurve(live.current.curve, i, shape));
    return [
      { label: t("automation.menu.linear"), onSelect: () => set({ type: "Linear" }) },
      { label: t("automation.menu.step"), onSelect: () => set({ type: "Step" }) },
      { label: t("automation.menu.smooth"), onSelect: () => set({ type: "Curve", tension: 0.5 }) },
    ];
  };

  const onContextMenu = (e: MouseEvent<SVGSVGElement>) => {
    const { curve: c, box: b } = live.current;
    const { x, y } = local(e);
    const hit = hitPoint(c.points, x, y, b.w, b.h, HIT);
    if (hit === null) {
      openContextMenu(e, shapeItems(segmentAt(c.points, Math.min(Math.max(x / b.w, 0), 1)).i));
      return;
    }
    openContextMenu(e, [
      ...(c.points.length > 1
        ? [{ label: t("devices.lfo_curve.delete"), onSelect: () => commit(removePoint(live.current.curve, hit)) }, "separator" as const]
        : []),
      ...shapeItems(hit),
    ]);
  };

  const onKeyDown = (i: number) => (e: KeyboardEvent) => {
    const c = live.current.curve;
    const p = c.points[i]!;
    const step = e.shiftKey ? 1 / 256 : 1 / live.current.divisions;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, e.shiftKey ? 0.01 : 0.05],
      ArrowDown: [0, e.shiftKey ? -0.01 : -0.05],
    };
    if (e.key === "Delete" || e.key === "Backspace") commit(removePoint(c, i));
    else if (moves[e.key]) commit(movePoint(c, i, p.x + moves[e.key]![0], p.y + moves[e.key]![1]));
    else return;
    e.preventDefault();
  };

  const { w: bw, h: bh } = box;
  return (
    <div
      ref={ref}
      className={clsx(
        "eth-plot eth-plot--interactive eth-lfo-curve",
        draft && "eth-lfo-curve--dragging",
        off && "eth-lfo-curve--off",
        band !== undefined && `eth-lfo-curve--band eth-lfo-band--${band}`,
      )}
      data-testid={testId}
      data-band={band}
    >
      <svg
        className="eth-plot__svg"
        width={bw}
        height={bh}
        viewBox={`0 0 ${bw} ${bh}`}
        role="img"
        aria-label={t("devices.lfo_curve.label", { count: curve.points.length })}
        data-points={curve.points.length}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onContextMenu={onContextMenu}
      >
        <title>{t("devices.lfo_curve.hint")}</title>
        <g className="eth-plot__grid" aria-hidden="true">
          {Array.from({ length: divisions - 1 }, (_, k) => {
            const x = ((k + 1) / divisions) * bw;
            return <line key={k} className={clsx((k + 1) % 4 === 0 && "eth-lfo-curve__beat")} x1={x} x2={x} y1={0} y2={bh} />;
          })}
          <line x1={0} x2={bw} y1={bh / 2} y2={bh / 2} />
        </g>
        {played && <polyline className="eth-lfo-curve__gain" points={curvePath(curve.points, bw, bh, played)} data-testid="lfo-curve-gain" />}
        <polygon className="eth-plot__fill" points={`0,${bh} ${curvePath(curve.points, bw, bh)} ${bw},${bh}`} />
        <polyline className="eth-plot__line" points={curvePath(curve.points, bw, bh)} data-testid="lfo-curve-line" />
        <line ref={playhead} className="eth-lfo-curve__playhead" x1={0} x2={0} y1={0} y2={bh} style={{ visibility: "hidden" }} />
        {curve.points.map((p, i) => (
          <circle
            key={i}
            className="eth-plot__handle eth-lfo-curve__point"
            cx={p.x * bw}
            cy={(1 - p.y) * bh}
            tabIndex={0}
            role="button"
            aria-label={t("devices.lfo_curve.point", { n: i + 1, x: Math.round(p.x * 100), y: Math.round(p.y * 100) })}
            data-point={i}
            onKeyDown={onKeyDown(i)}
          />
        ))}
      </svg>
    </div>
  );
}
