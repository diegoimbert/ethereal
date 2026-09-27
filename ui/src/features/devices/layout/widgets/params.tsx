/**
 * Param widgets of the catalog: Knob, Slider, Toggle, Choice, Number. Each binds one param
 * and renders inside `ParamShell` (MIDI learn, modulation slot, automation affordances).
 */

import type { KeyboardEvent, PointerEvent } from "react";
import { useRef } from "react";
import type { ParamUnit, WidgetSize } from "@/generated";
import { Button, Fader, Knob, NumberField, Select, clamp01 } from "@/kit";
import { labelIndex, labelValue } from "../../paramScale";
import type { ParamBinding } from "../context";
import { ParamShell } from "../ParamShell";
import { normalizedStep, stepDecimals, toNormalized } from "../values";

export interface ParamWidgetProps {
  binding: ParamBinding;
  size: WidgetSize;
  /** Caption (`LayoutItem.label`, else the param name). */
  label: string;
}

/** Kit size of a semantic widget size. */
export function kitSize(size: WidgetSize): "sm" | "md" | "lg" {
  return size === "Large" ? "lg" : size === "Medium" ? "md" : "sm";
}

export function KnobWidget({ binding: b, size, label }: ParamWidgetProps) {
  const s = kitSize(size);
  return (
    <ParamShell binding={b} kind="knob" className={`eth-param--${s}`}>
      <Knob
        size={s}
        value={b.normalized}
        defaultValue={toNormalized(b.info, b.info.default)}
        bipolar={b.bipolar}
        label={label}
        valueText={b.text}
        onChange={b.setNormalized}
        onChangeStart={b.begin}
        onChangeEnd={b.end}
      />
      {s !== "lg" && <span className="eth-param__value">{b.text}</span>}
    </ParamShell>
  );
}

export function SliderWidget({ binding: b, label, vertical }: ParamWidgetProps & { vertical: boolean }) {
  if (vertical) {
    return (
      <ParamShell binding={b} kind="slider-v">
        <Fader
          value={b.normalized}
          defaultValue={toNormalized(b.info, b.info.default)}
          label={label}
          valueText={b.text}
          onChange={b.setNormalized}
          onChangeStart={b.begin}
          onChangeEnd={b.end}
        />
        <span className="eth-param__value">{b.text}</span>
        <span className="eth-param__name">{label}</span>
      </ParamShell>
    );
  }
  return (
    <ParamShell binding={b} kind="slider">
      <span className="eth-param__row">
        <span className="eth-param__name">{label}</span>
        <span className="eth-param__value">{b.text}</span>
      </span>
      <HSlider binding={b} label={label} />
    </ParamShell>
  );
}

/** Horizontal slider: drag (Shift = fine) or click to jump, arrows, double-click resets. */
function HSlider({ binding: b, label }: { binding: ParamBinding; label: string }) {
  const drag = useRef<{ x: number; v: number; w: number } | null>(null);
  const v = clamp01(b.normalized);
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const r = el.getBoundingClientRect();
    const w = r.width || 1;
    b.begin();
    // Clicking the track jumps there; the thumb itself drags relatively.
    const jump = r.width > 0 && !(e.target as HTMLElement).classList.contains("eth-hslider__thumb");
    const start = jump ? clamp01((e.clientX - r.left) / w) : v;
    if (jump) b.setNormalized(start);
    drag.current = { x: e.clientX, v: start, w };
    e.preventDefault();
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const k = e.shiftKey ? 0.1 : 1;
    b.setNormalized(clamp01(d.v + ((e.clientX - d.x) / d.w) * k));
  };
  const onPointerUp = () => {
    if (!drag.current) return;
    drag.current = null;
    b.end();
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = normalizedStep(b.info) * (e.shiftKey ? 10 : 1);
    const next =
      e.key === "ArrowRight" || e.key === "ArrowUp"
        ? v + step
        : e.key === "ArrowLeft" || e.key === "ArrowDown"
          ? v - step
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? 1
              : null;
    if (next === null) return;
    e.preventDefault();
    b.setNormalized(clamp01(next));
  };
  const origin = b.bipolar ? toNormalized(b.info, 0) : 0;
  const lo = Math.min(origin, v);
  const hi = Math.max(origin, v);
  return (
    <div
      className="eth-hslider"
      role="slider"
      aria-orientation="horizontal"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={v}
      aria-valuetext={b.text}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onKeyDown={onKeyDown}
      onDoubleClick={b.reset}
    >
      <span className="eth-hslider__track">
        <span className="eth-hslider__fill" style={{ left: `${lo * 100}%`, width: `${(hi - lo) * 100}%` }} />
      </span>
      <span className="eth-hslider__thumb" style={{ left: `${v * 100}%` }} />
    </div>
  );
}

export function ToggleWidget({ binding: b, label }: ParamWidgetProps) {
  const labels = b.info.labels;
  const on = labels && labels.length >= 2 ? labelIndex(b.info, b.plain) >= 1 : b.plain >= (b.info.min + b.info.max) / 2;
  const onOff = labels && labels.length === 2 ? labels : ["Off", "On"];
  return (
    <ParamShell binding={b} kind="toggle">
      <Button size="sm" active={on} aria-label={label} onClick={() => b.setPlain(on ? b.info.min : b.info.max)}>
        {onOff[on ? 1 : 0]}
      </Button>
      <span className="eth-param__name">{label}</span>
    </ParamShell>
  );
}

/** Choices with at most this many labels (and short ones) render as a segmented control. */
export const SEGMENTED_MAX = 5;
/** Total characters of the labels a segmented control fits; longer sets use a dropdown. */
export const SEGMENTED_CHARS = 18;

export function isSegmented(labels: ReadonlyArray<string>): boolean {
  return labels.length <= SEGMENTED_MAX && labels.join("").length <= SEGMENTED_CHARS;
}

export function ChoiceWidget({ binding: b, label }: ParamWidgetProps) {
  const labels = b.info.labels ?? [];
  const index = labelIndex(b.info, b.plain);
  if (labels.length >= 2 && isSegmented(labels)) {
    return (
      <ParamShell binding={b} kind="choice" className="eth-param--segmented">
        <div className="eth-segmented" role="group" aria-label={label}>
          {labels.map((l, i) => (
            <Button
              key={i}
              size="sm"
              active={i === index}
              className="eth-segmented__item"
              onClick={() => b.setPlain(labelValue(b.info, i))}
            >
              {l}
            </Button>
          ))}
        </div>
        <span className="eth-param__name">{label}</span>
      </ParamShell>
    );
  }
  if (labels.length < 2) return <KnobWidget binding={b} size="Small" label={label} />;
  return (
    <ParamShell binding={b} kind="choice" as="label">
      <Select
        size="sm"
        className="eth-param__select"
        aria-label={label}
        value={String(index)}
        onChange={(v) => b.setPlain(labelValue(b.info, Number(v)))}
        options={labels.map((l, i) => ({ value: String(i), label: l }))}
      />
      <span className="eth-param__name">{label}</span>
    </ParamShell>
  );
}

const UNIT_SUFFIX: Record<ParamUnit, string | undefined> = {
  None: undefined,
  Decibels: "dB",
  Hertz: "Hz",
  Milliseconds: "ms",
  Seconds: "s",
  Percent: "%",
  Semitones: "st",
  Ratio: ": 1",
  Pan: undefined,
  Toggle: undefined,
};

export function NumberWidget({ binding: b, label }: ParamWidgetProps) {
  const step = b.info.step && b.info.step > 0 ? b.info.step : undefined;
  const precision = step ? stepDecimals(step) : 2;
  return (
    <ParamShell binding={b} kind="number">
      <NumberField
        size="sm"
        className="eth-param__number"
        aria-label={label}
        value={b.plain}
        min={Math.min(b.info.min, b.info.max)}
        max={Math.max(b.info.min, b.info.max)}
        step={step ?? Math.abs(b.info.max - b.info.min) / 100}
        precision={precision}
        unit={UNIT_SUFFIX[b.info.unit]}
        onChange={b.setPlain}
        onChangeStart={b.begin}
        onChangeEnd={b.end}
      />
      <span className="eth-param__name">{label}</span>
    </ParamShell>
  );
}
