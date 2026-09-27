import clsx from "clsx";
import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { setDragCursor } from "@/kit";

export interface CommitFieldProps {
  /** Current value as text (shown while not editing). */
  value: string;
  /** Called with the edited text on Enter/blur. Return `false` to reject it (reverts). */
  onCommit(text: string): boolean | void;
  /** Arrow up/down (sign = direction, `fine` with Shift). */
  onStep?(direction: 1 | -1, fine: boolean): void;
  label: string;
  title?: string;
  disabled?: boolean;
  className?: string;
  width?: string;
  /**
   * Hold and drag vertically to change the value (a press without a drag still focuses the
   * field for typing). `onMove` gets the pixels dragged upward since the press.
   */
  drag?: {
    onStart(): void;
    onMove(upPx: number, fine: boolean): void;
    onEnd(): void;
  };
}

const DRAG_THRESHOLD_PX = 3;

/**
 * A text field that edits a value committed to the engine: Enter or blur commits, Escape
 * reverts, ArrowUp/ArrowDown step, and (with `drag`) hold-and-drag up/down changes it.
 * While not focused it shows `value`.
 */
export function CommitField({ value, onCommit, onStep, label, title, disabled, className, width, drag }: CommitFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const [invalid, setInvalid] = useState(false);
  const press = useRef<{ id: number; y: number; dragging: boolean } | null>(null);

  const onPointerDown = (e: PointerEvent<HTMLInputElement>) => {
    const el = e.currentTarget;
    if (!drag || e.button !== 0 || disabled || document.activeElement === el) return;
    // Don't focus yet: this may become a drag. A plain click focuses on release.
    e.preventDefault();
    el.setPointerCapture?.(e.pointerId);
    press.current = { id: e.pointerId, y: e.clientY, dragging: false };
  };

  const onPointerMove = (e: PointerEvent<HTMLInputElement>) => {
    const p = press.current;
    if (!drag || !p || p.id !== e.pointerId) return;
    const up = p.y - e.clientY;
    if (!p.dragging) {
      if (Math.abs(up) < DRAG_THRESHOLD_PX) return;
      p.dragging = true;
      setDragCursor("ns-resize");
      drag.onStart();
    }
    drag.onMove(up, e.shiftKey);
  };

  const onPointerUp = (e: PointerEvent<HTMLInputElement>) => {
    const p = press.current;
    if (!p || p.id !== e.pointerId) return;
    press.current = null;
    if (p.dragging) {
      setDragCursor(null);
      drag?.onEnd();
    } else if (e.type === "pointerup") {
      e.currentTarget.focus();
    }
  };

  const commit = () => {
    if (draft === null) return;
    if (draft !== value && onCommit(draft) === false) setInvalid(true);
    else setInvalid(false);
    setDraft(null);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      commit();
      e.currentTarget.blur();
    } else if (e.key === "Escape") {
      setDraft(null);
      setInvalid(false);
      e.currentTarget.blur();
    } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && onStep) {
      e.preventDefault();
      setDraft(null);
      onStep(e.key === "ArrowUp" ? 1 : -1, e.shiftKey);
    }
  };

  return (
    <input
      type="text"
      inputMode="decimal"
      className={clsx("eth-tb-field", drag && "eth-tb-field--draggable", invalid && "eth-tb-field--invalid", className)}
      style={width ? { width } : undefined}
      aria-label={label}
      aria-invalid={invalid || undefined}
      title={title}
      disabled={disabled}
      value={draft ?? value}
      onFocus={(e) => {
        setDraft(value);
        e.currentTarget.select();
      }}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    />
  );
}
