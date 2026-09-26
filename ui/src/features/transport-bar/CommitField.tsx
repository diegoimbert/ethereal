import clsx from "clsx";
import { useState, type KeyboardEvent } from "react";

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
}

/**
 * A text field that edits a value committed to the engine: Enter or blur commits, Escape
 * reverts, ArrowUp/ArrowDown step. While not focused it shows `value`.
 */
export function CommitField({ value, onCommit, onStep, label, title, disabled, className, width }: CommitFieldProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const [invalid, setInvalid] = useState(false);

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
      className={clsx("eth-tb-field", invalid && "eth-tb-field--invalid", className)}
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
    />
  );
}
