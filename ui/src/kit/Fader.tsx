import clsx from "clsx";
import { useVerticalDrag } from "./useVerticalDrag";

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
  /** Pixel height. */
  height?: number;
  label?: string;
  valueText?: string;
  disabled?: boolean;
  className?: string;
}

/** Vertical fader. Drag to change (Shift = fine), double-click to reset. */
export function Fader({
  value,
  onChange,
  onChangeStart,
  onChangeEnd,
  defaultValue,
  height = 120,
  label,
  valueText,
  disabled = false,
  className,
}: FaderProps) {
  const handlers = useVerticalDrag({
    value,
    onChange: disabled ? undefined : onChange,
    sensitivity: 1 / height,
    defaultValue,
    onChangeStart,
    onChangeEnd,
  });
  const v = Math.min(1, Math.max(0, value));
  return (
    <div
      className={clsx("eth-fader", className)}
      style={{ height }}
      role="slider"
      aria-orientation="vertical"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={1}
      aria-valuenow={v}
      aria-valuetext={valueText}
      aria-disabled={disabled || undefined}
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
