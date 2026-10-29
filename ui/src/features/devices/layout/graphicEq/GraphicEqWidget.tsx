/**
 * The graphic EQ's editor (`Widget::GraphicEq`): the response of the used bands on a
 * log-frequency (10 Hz..30 kHz) / dB grid over the device's input and output spectrum,
 * filling its container (it grows with the device window).
 *
 * By default no band is used: a flat line. Gestures:
 * - double-click (anywhere, typically on the line): a free band becomes a bell there, at that
 *   gain; keep the second press down and move to drag it at once;
 * - drag a point: frequency (x) and gain (y; Q for shapes without gain); Alt-drag = Q,
 *   Shift = fine;
 * - wheel over a point: Q (Shift = fine);
 * - double-click a point (or Delete on a focused one): the band is freed;
 * - right-click a point: type, slope, on/off, delete; right-click the background: remove
 *   every band;
 * - click a point: its panel (type, frequency, gain, Q, slope, on, delete) opens under the
 *   curve; Escape or a click on the background closes it.
 *
 * Every gesture (a placement and its drag, a drag, a wheel burst, a removal) is one undo
 * step: the gesture sender groups its `SetParam`s.
 */

import clsx from "clsx";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type PointerEvent } from "react";
import { Trash2 } from "lucide-react";
import type { Widget, WidgetSize } from "@/generated";
import { Button, IconButton, openContextMenu, type ContextMenuEntry } from "@/kit";
import { inputSettings, readWheel, wheelScrollPx } from "@/timeline";
import { createDoublePress } from "@/timeline/doublePress";
import { t as tr, useT } from "@/i18n";
import { bindParam, useLayoutContext, type LayoutContextValue, type ParamBinding } from "../context";
import { useBoxSize } from "../useBoxSize";
import { TypedFrame } from "../widgets/typed";
import { ChoiceWidget, KnobWidget, ToggleWidget } from "../widgets/params";
import { spectrumY, tiltedDb } from "../eq/geometry";
import { useEngineSampleRate, useEqSpectra } from "../eq/hooks";
import {
  DEFAULT_RANGE,
  FREQ_LABELS,
  FREQ_TICKS,
  RANGES,
  axisFreqs,
  dbAt,
  freeSlot,
  freqAt,
  freqLabel,
  freqToT,
  handleAt,
  hitBand,
  pxPerDb,
  qAfter,
  resolveBand,
  tToFreq,
  xOf,
  yOf,
  type BandDraft,
  type Box,
  type ResolvedBand,
} from "./geometry";
import { SHAPES, bandCurve, usesGain, usesQ, usesSlope } from "./response";
import "./graphicEq.css";

type Spec = Extract<Widget, { type: "GraphicEq" }>;

export interface GraphicEqProps {
  widget: Spec;
  size: WidgetSize;
  label: string | null;
}

/** Curve sample points (log-spaced: point `i` sits at `x = i / (N - 1) · width`). */
const POINTS = 320;
const FREQS = axisFreqs(POINTS);
/** Wheel bursts closer than this share one undo step. */
const WHEEL_GESTURE_MS = 400;
/** A placement this close (px) to the 0 dB line lands on it. */
const SNAP_ZERO_PX = 4;
/** Band colours (`eth-geq__band--N`). */
const COLOURS = 8;

const clamp = (v: number, lo: number, hi: number) => (v < lo ? lo : v > hi ? hi : v);

type Grab = { index: number; mode: "move" | "q"; x: number; y: number; ft: number; gain: number; q: number };

/** "bell · 1.00 khz · +3.0 db · q 1.00" (what a band does). */
function readout(b: ResolvedBand): string {
  const { kind, freq, gain, q, slope } = b.bindings;
  const parts = [kind.text, freq.text];
  if (usesGain(b.shape)) parts.push(gain.text);
  if (usesQ(b.shape)) parts.push(`Q ${q.text}`);
  if (usesSlope(b.shape)) parts.push(slope.text);
  if (!b.on) parts.push(tr("common.off"));
  return parts.join(" · ");
}

/** One gesture: `f` between `begin` and `end` (one undo step). */
function gesture(ctx: LayoutContextValue, f: () => void): void {
  ctx.sender.begin();
  f();
  ctx.sender.end();
}

/** Free a band (its params stay: a new placement sets them all). */
function removeBand(ctx: LayoutContextValue, b: ResolvedBand): void {
  gesture(ctx, () => b.bindings.used.setPlain(0));
}

export function GraphicEqWidget({ widget: w, size, label }: GraphicEqProps) {
  const t = useT();
  const ctx = useLayoutContext();
  const { sender } = ctx;
  const sampleRate = useEngineSampleRate();
  const spectra = useEqSpectra(ctx.device.id, w.spectrum !== "None");
  const [range, setRange] = useState(DEFAULT_RANGE);
  const [selected, setSelected] = useState<number | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const [dragging, setDragging] = useState(false);
  // What a gesture sent, drawn until the document shows it.
  const [drafts, setDrafts] = useState<Readonly<Record<number, BandDraft>>>({});
  const [ref, box] = useBoxSize<HTMLDivElement>();

  const bind = (id: number): ParamBinding | null => {
    const info = ctx.params.get(id);
    return info ? bindParam(ctx, info) : null;
  };
  const bands = w.bands.flatMap((spec, i) => {
    const b = resolveBand(i, spec, bind, drafts[i]);
    return b ? [b] : [];
  });
  const output = bind(w.output);
  const used = bands.filter((b) => b.used);

  const grab = useRef<Grab | null>(null);
  const double = useRef(createDoublePress());
  const wheel = useRef<{ timer: ReturnType<typeof setTimeout> | null }>({ timer: null });
  // Slots placed but not yet used in the document (two quick placements take two slots).
  const placing = useRef(new Set<number>());
  const live = useRef({ bands, box, range, selected });
  useLayoutEffect(() => {
    live.current = { bands, box, range, selected };
  });

  // The document caught up (or changed under the drafts): draw the document.
  useEffect(() => {
    if (grab.current) return;
    setDrafts((d) => (Object.keys(d).length === 0 ? d : {}));
    placing.current.clear();
  }, [ctx.device.params]);

  // A freed band can't stay selected.
  const selectedBand = bands.find((b) => b.index === selected && b.used);
  useEffect(() => {
    if (selected !== null && !selectedBand && !grab.current) setSelected(null);
  }, [selected, selectedBand]);

  const endWheel = () => {
    if (wheel.current.timer === null) return;
    clearTimeout(wheel.current.timer);
    wheel.current.timer = null;
    sender.end();
  };

  // Response of each used band at the sample points; recomputed when a band changes.
  const key = used.map((b) => `${b.index}:${b.shape}:${b.freq}:${b.gainDb}:${b.q}:${b.slope}:${b.on ? 1 : 0}`).join("|");
  const curves = useMemo(() => new Map(used.map((b) => [b.index, bandCurve(b, FREQS, sampleRate)] as const)), [key, sampleRate]); // eslint-disable-line react-hooks/exhaustive-deps
  const sum = useMemo(() => {
    const out = new Array<number>(POINTS).fill(0);
    for (const c of curves.values()) for (let i = 0; i < POINTS; i++) out[i]! += c[i]!;
    return out;
  }, [curves]);

  /** Write one band param (draft first, so the drag follows the pointer). */
  const write = (b: ResolvedBand, draft: BandDraft, sends: ReadonlyArray<[ParamBinding, number]>) => {
    setDrafts((d) => ({ ...d, [b.index]: { ...d[b.index], ...draft } }));
    for (const [p, v] of sends) p.setPlain(v);
  };

  // Wheel = Q on the band under the pointer (native listener: React's is passive).
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      const r = el.getBoundingClientRect();
      const { bands: bs, box: bx, range: rg, selected: sel } = live.current;
      const hit = hitBand(bx, bs, e.clientX - r.left, e.clientY - r.top, rg, sel);
      const b = hit === null ? undefined : bs.find((x) => x.index === hit);
      if (!b || !usesQ(b.shape)) return;
      e.preventDefault();
      if (wheel.current.timer === null) sender.begin();
      else clearTimeout(wheel.current.timer);
      wheel.current.timer = setTimeout(() => {
        wheel.current.timer = null;
        sender.end();
      }, WHEEL_GESTURE_MS);
      setSelected(b.index);
      const n = readWheel(e, el);
      const d = wheelScrollPx(n.dy || n.dx, "y", inputSettings());
      const info = b.bindings.q.info;
      const q = clamp(qAfter(b.q, d * (e.shiftKey ? 0.05 : 0.25)), info.min, info.max);
      write(b, { q }, [[b.bindings.q, q]]);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      el.removeEventListener("wheel", onWheel);
      endWheel();
    };
  }, [sender]); // eslint-disable-line react-hooks/exhaustive-deps

  const local = (e: { clientX: number; clientY: number; currentTarget: Element }) => {
    const r = e.currentTarget.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };

  /** A free band becomes a bell at `(x, y)`, inside the open gesture. */
  const place = (x: number, y: number): ResolvedBand | null => {
    const { bands: bs, box: bx, range: rg } = live.current;
    const index = freeSlot(bs, placing.current);
    const b = index === null ? undefined : bs.find((v) => v.index === index);
    if (!b) return null;
    placing.current.add(b.index);
    const { used: u, on, kind, freq, gain, q, slope } = b.bindings;
    const hz = clamp(freqAt(bx, x), freq.info.min, freq.info.max);
    const near0 = Math.abs(y - yOf(bx, 0, rg)) <= SNAP_ZERO_PX;
    const db = near0 ? 0 : Math.round(clamp(dbAt(bx, y, rg), gain.info.min, gain.info.max) * 10) / 10;
    write(b, { used: true, on: true, shape: "Bell", freq: hz, gainDb: db, q: q.info.default, slope: slope.info.default }, [
      [u, u.info.max],
      [on, on.info.max],
      [kind, SHAPES.indexOf("Bell")],
      [freq, hz],
      [gain, db],
      [q, q.info.default],
      [slope, slope.info.default],
    ]);
    return { ...b, used: true, on: true, shape: "Bell", freq: hz, gainDb: db, q: q.info.default };
  };

  const startGrab = (b: ResolvedBand, x: number, y: number, alt: boolean) => {
    grab.current = { index: b.index, mode: alt ? "q" : "move", x, y, ft: freqToT(b.freq), gain: b.gainDb, q: b.q };
    setSelected(b.index);
    setDragging(true);
  };

  const onPointerDown = (e: PointerEvent<SVGSVGElement>) => {
    if (e.button !== 0) return;
    endWheel();
    const { bands: bs, box: bx, range: rg, selected: sel } = live.current;
    const { x, y } = local(e);
    const hit = hitBand(bx, bs, x, y, rg, sel);
    const isDouble = double.current(e, hit === null ? "empty" : `band:${hit}`);
    e.preventDefault();
    const target = hit === null ? undefined : bs.find((b) => b.index === hit);
    if (target && isDouble) {
      removeBand(ctx, target);
      setSelected(null);
      return;
    }
    if (!target && !isDouble) {
      setSelected(null);
      return;
    }
    sender.begin();
    const b = target ?? place(x, y);
    if (!b) {
      sender.end();
      return;
    }
    startGrab(b, x, y, e.altKey);
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<SVGSVGElement>) => {
    const g = grab.current;
    const { bands: bs, box: bx, range: rg, selected: sel } = live.current;
    const { x, y } = local(e);
    if (!g) {
      const hit = hitBand(bx, bs, x, y, rg, sel);
      if (hit !== hover) setHover(hit);
      return;
    }
    const b = bs.find((v) => v.index === g.index);
    if (!b) return;
    // Incremental, so Shift (fine) can be pressed and released mid-drag.
    const fine = e.shiftKey ? 0.1 : 1;
    const dx = (x - g.x) * fine;
    const dy = (y - g.y) * fine;
    g.x = x;
    g.y = y;
    const { freq, gain, q } = b.bindings;
    const qInfo = q.info;
    if (g.mode === "q") {
      if (!usesQ(b.shape)) return;
      g.q = clamp(qAfter(g.q, dy), qInfo.min, qInfo.max);
      write(b, { q: g.q }, [[q, g.q]]);
      return;
    }
    g.ft = clamp(g.ft + dx / Math.max(1, bx.w), freqToT(freq.info.min), freqToT(freq.info.max));
    const hz = tToFreq(g.ft);
    const draft: BandDraft = { freq: hz };
    const sends: [ParamBinding, number][] = [[freq, hz]];
    if (usesGain(b.shape)) {
      g.gain = clamp(g.gain - dy / pxPerDb(bx, rg), gain.info.min, gain.info.max);
      draft.gainDb = g.gain;
      sends.push([gain, g.gain]);
    } else if (usesQ(b.shape) && dy !== 0) {
      // No gain: vertical = Q (up = narrower / more resonant).
      g.q = clamp(qAfter(g.q, dy), qInfo.min, qInfo.max);
      draft.q = g.q;
      sends.push([q, g.q]);
    }
    write(b, draft, sends);
  };

  const onPointerUp = () => {
    if (!grab.current) return;
    grab.current = null;
    setDragging(false);
    sender.end();
  };

  const menuFor = (b: ResolvedBand): ContextMenuEntry[] => {
    const { kind, slope, on } = b.bindings;
    const set = (p: ParamBinding, v: number) => gesture(ctx, () => p.setPlain(v));
    const labels = kind.info.labels ?? [];
    const slopes = slope.info.labels ?? [];
    return [
      ...labels.map((l, i) => ({
        label: `${SHAPES[i] === b.shape ? "✓ " : ""}${l}`,
        onSelect: () =>
          gesture(ctx, () => {
            kind.setPlain(i);
            // A cut's resonance starts flat (Butterworth).
            const s = SHAPES[i];
            if ((s === "LowCut" || s === "HighCut") && b.shape !== s) b.bindings.q.setPlain(Math.SQRT1_2);
          }),
      })),
      "separator" as const,
      ...(usesSlope(b.shape)
        ? [
            {
              label: t("devices.graphic_eq.slope"),
              onSelect: () => undefined,
              submenu: slopes.map((l, i) => ({ label: `${i === b.slope ? "✓ " : ""}${l}`, onSelect: () => set(slope, i) })),
            },
          ]
        : []),
      { label: b.on ? t("devices.graphic_eq.disable_band") : t("devices.graphic_eq.enable_band"), onSelect: () => set(on, b.on ? 0 : 1) },
      { label: t("devices.graphic_eq.delete_band"), danger: true, onSelect: () => removeBand(ctx, b) },
    ];
  };

  const onContextMenu = (e: MouseEvent<SVGSVGElement>) => {
    const { bands: bs, box: bx, range: rg, selected: sel } = live.current;
    const { x, y } = local(e);
    const hit = hitBand(bx, bs, x, y, rg, sel);
    const b = hit === null ? undefined : bs.find((v) => v.index === hit);
    if (b) {
      setSelected(b.index);
      openContextMenu(e, menuFor(b));
      return;
    }
    const inUse = bs.filter((v) => v.used);
    openContextMenu(
      e,
      inUse.length === 0
        ? []
        : [{ label: t("devices.graphic_eq.remove_all"), danger: true, onSelect: () => gesture(ctx, () => inUse.forEach((v) => v.bindings.used.setPlain(0))) }],
    );
  };

  const onKeyDown = (b: ResolvedBand) => (e: KeyboardEvent) => {
    const fine = e.shiftKey ? 0.1 : 1;
    const { freq, gain, q, on } = b.bindings;
    switch (e.key) {
      case "ArrowLeft":
      case "ArrowRight": {
        const dir = e.key === "ArrowLeft" ? -1 : 1;
        gesture(ctx, () => freq.setPlain(tToFreq(freqToT(b.freq) + (dir * fine) / 80)));
        break;
      }
      case "ArrowUp":
      case "ArrowDown": {
        const dir = e.key === "ArrowDown" ? -1 : 1;
        if (e.altKey || !usesGain(b.shape)) {
          if (!usesQ(b.shape)) return;
          gesture(ctx, () => q.setPlain(clamp(qAfter(b.q, -dir * 12 * fine), q.info.min, q.info.max)));
        } else gesture(ctx, () => gain.setPlain(clamp(b.gainDb + dir * fine, gain.info.min, gain.info.max)));
        break;
      }
      case "Enter":
        gesture(ctx, () => on.setPlain(b.on ? 0 : 1));
        break;
      case "Delete":
      case "Backspace":
        removeBand(ctx, b);
        setSelected(null);
        break;
      case "Escape":
        setSelected(null);
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  const shown = bands.find((b) => b.used && b.index === (dragging ? selected : (hover ?? selected)));
  const nextRange = RANGES[(RANGES.indexOf(range) + 1) % RANGES.length] ?? DEFAULT_RANGE;

  return (
    <TypedFrame type="graphic-eq" size={size} label={label}>
      <div className={clsx("eth-geq", dragging && "eth-geq--dragging", hover !== null && "eth-geq--over-handle")} data-testid="widget-graphic-eq">
        <div className="eth-geq__bar">
          <div className="eth-geq__info" aria-live="polite">
            {shown ? (
              <span className={clsx("eth-geq__readout", `eth-geq__band--${shown.index % COLOURS}`)} data-testid="geq-readout" data-band={shown.index}>
                {readout(shown)}
              </span>
            ) : used.length === 0 ? (
              <span className="eth-geq__hint">{t("devices.graphic_eq.hint")}</span>
            ) : null}
          </div>
          <Button
            size="sm"
            tone="ghost"
            className="eth-geq__range"
            title={t("devices.graphic_eq.range_next", { db: nextRange })}
            aria-label={t("devices.graphic_eq.range", { db: range })}
            onClick={() => setRange(nextRange)}
            data-testid="geq-range"
          >
            {t("devices.graphic_eq.range_label", { db: range })}
          </Button>
          {output && (
            <div className="eth-geq__output">
              <KnobWidget binding={output} size="Small" label={output.info.name} />
            </div>
          )}
        </div>
        <div ref={ref} className="eth-geq__plot">
          <svg
            className="eth-geq__svg"
            width={box.w}
            height={box.h}
            viewBox={`0 0 ${box.w} ${box.h}`}
            role="img"
            aria-label={t("devices.graphic_eq.curve", { count: used.length })}
            data-bands={used.length}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={onPointerUp}
            onPointerLeave={() => setHover(null)}
            onContextMenu={onContextMenu}
          >
            <Graphics
              box={box}
              range={range}
              bands={used}
              curves={curves}
              sum={sum}
              selected={selected}
              hover={hover}
              spectra={w.spectrum === "None" ? null : spectra}
              showPre={w.spectrum === "PrePost"}
              onKeyDown={onKeyDown}
              onFocus={setSelected}
            />
          </svg>
          {selectedBand && !dragging && (
            <BandPanel band={selectedBand} box={box} range={range} onDelete={() => (removeBand(ctx, selectedBand), setSelected(null))} />
          )}
        </div>
      </div>
    </TypedFrame>
  );
}

/** The selected band's controls, under its point (above it when the point is low). */
function BandPanel({ band: b, box, range, onDelete }: { band: ResolvedBand; box: Box; range: number; onDelete(): void }) {
  const t = useT();
  const { kind, freq, gain, q, slope, on } = b.bindings;
  const h = handleAt(box, b, range);
  const below = h.y < box.h / 2;
  const left = clamp(h.x / Math.max(1, box.w), 0, 1) * 100;
  return (
    <div
      className={clsx("eth-geq__panel", below ? "eth-geq__panel--below" : "eth-geq__panel--above", `eth-geq__band--${b.index % COLOURS}`)}
      style={{ left: `${left}%`, top: `${h.y}px`, transform: `translateX(-${left}%)` }}
      role="group"
      aria-label={t("devices.graphic_eq.band", { n: b.index + 1 })}
      data-testid="geq-panel"
      data-band={b.index}
      onPointerDown={(e) => e.stopPropagation()}
    >
      <ChoiceWidget binding={kind} size="Small" label={kind.info.name} />
      <KnobWidget binding={freq} size="Small" label={freq.info.name} />
      {usesGain(b.shape) && <KnobWidget binding={gain} size="Small" label={gain.info.name} />}
      {usesQ(b.shape) && <KnobWidget binding={q} size="Small" label={q.info.name} />}
      {usesSlope(b.shape) && <ChoiceWidget binding={slope} size="Small" label={slope.info.name} />}
      <ToggleWidget binding={on} size="Small" label={on.info.name} />
      <IconButton size="sm" tone="ghost" label={t("devices.graphic_eq.delete_band")} icon={<Trash2 />} onClick={onDelete} data-testid="geq-delete" />
    </div>
  );
}

interface GraphicsProps {
  box: Box;
  range: number;
  bands: ReadonlyArray<ResolvedBand>;
  curves: ReadonlyMap<number, ReadonlyArray<number>>;
  sum: ReadonlyArray<number>;
  selected: number | null;
  hover: number | null;
  spectra: ReturnType<typeof useEqSpectra> | null;
  showPre: boolean;
  onKeyDown: (b: ResolvedBand) => (e: KeyboardEvent) => void;
  onFocus: (index: number) => void;
}

/** SVG points of a dB curve sampled at `FREQS`. */
function curvePoints(box: Box, db: ReadonlyArray<number>, range: number): string {
  const n = db.length;
  let s = "";
  for (let i = 0; i < n; i++) s += `${((i / (n - 1)) * box.w).toFixed(1)},${yOf(box, db[i]!, range).toFixed(1)} `;
  return s;
}

/** SVG points of a spectrum frame (log-spaced bins from `min_hz` to `max_hz`), tilted. */
function spectrumPoints(box: Box, f: { min_hz: number; max_hz: number; bins_db: ReadonlyArray<number> }): string {
  const n = f.bins_db.length;
  if (n < 2 || !(f.min_hz > 0) || !(f.max_hz > f.min_hz)) return "";
  let s = "";
  for (let i = 0; i < n; i++) {
    const hz = f.min_hz * Math.pow(f.max_hz / f.min_hz, (i + 0.5) / n);
    s += `${xOf(box, hz).toFixed(1)},${spectrumY(box, tiltedDb(f.bins_db[i]!, hz)).toFixed(1)} `;
  }
  return s;
}

/** dB grid lines for a display range. */
function dbTicks(range: number): number[] {
  const step = range <= 6 ? 2 : range <= 12 ? 3 : 6;
  const out: number[] = [];
  for (let d = -range + step; d < range; d += step) out.push(d);
  return out;
}

function Graphics({ box, range, bands, curves, sum, selected, hover, spectra, showPre, onKeyDown, onFocus }: GraphicsProps) {
  const zero = yOf(box, 0, range);
  const focus = bands.find((b) => b.index === (hover ?? selected));
  const focusCurve = focus && focus.on ? curves.get(focus.index) : undefined;
  const pre = spectra && showPre && spectra.pre ? spectrumPoints(box, spectra.pre) : "";
  const post = spectra?.post ? spectrumPoints(box, spectra.post) : "";
  const ticks = dbTicks(range);
  return (
    <>
      <g className="eth-geq__grid" aria-hidden="true">
        {FREQ_TICKS.map((f) => (
          <line key={`f${f}`} className={clsx(FREQ_LABELS.includes(f) && "eth-geq__grid--major")} x1={xOf(box, f)} x2={xOf(box, f)} y1={0} y2={box.h} />
        ))}
        {ticks.map((d) => (
          <line key={`d${d}`} className={clsx(d === 0 && "eth-geq__grid--zero")} x1={0} x2={box.w} y1={yOf(box, d, range)} y2={yOf(box, d, range)} />
        ))}
      </g>
      {pre && <polygon className="eth-geq__spectrum-pre" points={`${pre} ${box.w},${box.h} 0,${box.h}`} data-testid="geq-spectrum-pre" />}
      {post && (
        <>
          <polygon className="eth-geq__spectrum-post-fill" points={`${post} ${box.w},${box.h} 0,${box.h}`} />
          <polyline className="eth-geq__spectrum-post" points={post} data-testid="geq-spectrum-post" />
        </>
      )}
      <g className="eth-geq__labels" aria-hidden="true">
        {FREQ_LABELS.map((f) => (
          <text key={`f${f}`} className="eth-geq__label eth-geq__label--freq" x={xOf(box, f)} y={box.h - 2}>
            {freqLabel(f)}
          </text>
        ))}
        {ticks
          .filter((d) => d % (range <= 6 ? 2 : range <= 12 ? 6 : 12) === 0)
          .map((d) => (
            <text key={`d${d}`} className="eth-geq__label eth-geq__label--db" x={4} y={yOf(box, d, range)}>
              {d > 0 ? `+${d}` : d}
            </text>
          ))}
      </g>
      {focusCurve && (
        <polygon
          className={clsx("eth-geq__band-fill", `eth-geq__band--${focus!.index % COLOURS}`)}
          points={`0,${zero} ${curvePoints(box, focusCurve, range)} ${box.w},${zero}`}
          data-testid="geq-band-curve"
        />
      )}
      <polyline className="eth-geq__sum" points={curvePoints(box, sum, range)} data-testid="geq-sum" />
      {bands.map((b) => {
        const { x, y } = handleAt(box, b, range);
        return (
          <g
            key={b.index}
            className={clsx(
              "eth-geq__handle",
              `eth-geq__band--${b.index % COLOURS}`,
              !b.on && "eth-geq__handle--off",
              b.index === selected && "eth-geq__handle--selected",
            )}
            transform={`translate(${x.toFixed(1)} ${y.toFixed(1)})`}
            tabIndex={0}
            role="button"
            aria-label={`${tr("devices.graphic_eq.band", { n: b.index + 1 })}: ${readout(b)}`}
            aria-pressed={b.index === selected}
            data-band={b.index}
            data-on={b.on ? "true" : "false"}
            onKeyDown={onKeyDown(b)}
            onFocus={() => onFocus(b.index)}
          >
            <circle className="eth-geq__handle-dot" r={7} />
            <text className="eth-geq__handle-num">{b.index + 1}</text>
          </g>
        );
      })}
    </>
  );
}
