import clsx from "clsx";
import { size } from "../theme/tokens";
import { useVerticalDrag } from "./useVerticalDrag";

/** Used for drag sensitivity when the element has no layout (tests, hidden). */
const FALLBACK_HEIGHT = Number.parseFloat(size.faderHeight);

export interface FaderProps {
  /** Normalized value, 0..1 (mapping to dB is the caller's job). */
  value: number;
  onChange?: (value: number) => void;
  /** Pointer drag started (open an undo gesture). */
  onChangeStart?: () => void;
  /** Pointer drag ended (close the gesture). */
  onChangeEnd?: () => void;
  /** Double-click resets to this value. */
  defaultValue?: number;
  /** Pixel height for one-offs; default is the `--fader-height` token. */
  height?: number;
  label?: string;
  valueText?: string;
  disabled?: boolean;
  className?: string;
  /** MIDI learn hook (`midiTarget()` in features/midi-learn): marks this control as mappable. */
  "data-midi-target"?: string;
}

/** Vertical fader. Drag to change (Shift = fine), double-click to reset. */
export function Fader({
  value,
  onChange,
  onChangeStart,
  onChangeEnd,
  defaultValue,
  height,
  label,
  valueText,
  disabled = false,
  className,
  "data-midi-target": midiTarget,
}: FaderProps) {
  const handlers = useVerticalDrag({
    value,
    onChange: disabled ? undefined : onChange,
    // A full-height drag sweeps the whole range, whatever height the tokens give the fader.
    sensitivity: (el) => 1 / (el.clientHeight || height || FALLBACK_HEIGHT),
    defaultValue,
    onChangeStart,
    onChangeEnd,
  });
  const v = Math.min(1, Math.max(0, value));
  return (
    <div
      className={clsx("eth-fader", className)}
      style={height !== undefined ? { height } : undefined}
      role="slider"
      aria-orientation="vertical"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={v}
      aria-valuetext={valueText}
      aria-disabled={disabled || undefined}
      data-midi-target={midiTarget}
      title={valueText}
      {...handlers}
    >
      <div className="eth-fader__track">
        <div className="eth-fader__fill" style={{ height: `${v * 100}%` }} />
      </div>
      <div className="eth-fader__thumb" style={{ bottom: `${v * 100}%` }} />
    </div>
  );
}
