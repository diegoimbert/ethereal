import clsx from "clsx";
import { Check, ChevronDown } from "lucide-react";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import type { Size } from "./variants";

export interface SelectOption<V extends string> {
  value: V;
  label: string;
  disabled?: boolean;
  /** Options with the same group are listed under that heading (in first-seen order). */
  group?: string;
  /** Optional leading icon in the list. */
  icon?: ReactNode;
}

export interface SelectProps<V extends string>
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "value" | "onChange" | "children"> {
  options: ReadonlyArray<SelectOption<V>>;
  /** The selected value; one not among the options shows `placeholder`. */
  value: V;
  onChange: (value: V) => void;
  size?: Size;
  /** Shown when `value` matches no option (e.g. "+ Show parameter…"). */
  placeholder?: string;
}

/** Keep the list inside the window. */
const VIEWPORT_MARGIN = 4;
/** Must match the close animation (`--select-exit-duration`); unmounts if no animationend. */
const CLOSE_MS = 120;

/**
 * Drop-down select: a button showing the current option and our own animated list (not the
 * system one). Keyboard: ↑/↓ or Enter/Space opens; ↑/↓, Home/End and typing a letter move;
 * Enter/Space picks; Escape or Tab closes. ARIA: a `combobox` button controlling a `listbox`.
 */
export function Select<V extends string>({
  options,
  value,
  onChange,
  size = "md",
  placeholder,
  className,
  disabled,
  ...rest
}: SelectProps<V>) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const [state, setState] = useState<"closed" | "open" | "closing">("closed");
  const [active, setActive] = useState(-1);
  const current = options.find((o) => o.value === value);
  const enabled = options.flatMap((o, i) => (o.disabled ? [] : [i]));

  const open = () => {
    if (disabled) return;
    setActive(Math.max(0, options.findIndex((o) => o.value === value)));
    setState("open");
  };
  const close = (refocus = true) => {
    setState((s) => (s === "open" ? "closing" : s));
    if (refocus) trigger.current?.focus({ preventScroll: true });
  };
  const pick = (i: number) => {
    const o = options[i];
    if (!o || o.disabled) return;
    close();
    if (o.value !== value) onChange(o.value);
  };
  const move = (from: number, dir: 1 | -1) => {
    if (!enabled.length) return from;
    const at = enabled.indexOf(from);
    return enabled[at < 0 ? (dir > 0 ? 0 : enabled.length - 1) : (at + dir + enabled.length) % enabled.length]!;
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const isOpen = state === "open";
    if (!isOpen) {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
        e.preventDefault();
        open();
      }
      return;
    }
    e.stopPropagation();
    if (e.key === "Escape" || e.key === "Tab") {
      if (e.key === "Escape") e.preventDefault();
      close(e.key === "Escape");
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => move(a, e.key === "ArrowDown" ? 1 : -1));
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      setActive(e.key === "Home" ? (enabled[0] ?? -1) : (enabled[enabled.length - 1] ?? -1));
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      pick(active);
    } else if (e.key.length === 1 && /\S/.test(e.key)) {
      // Type-ahead: the next option starting with that letter.
      const k = e.key.toLowerCase();
      const order = [...enabled.filter((i) => i > active), ...enabled.filter((i) => i <= active)];
      const hit = order.find((i) => options[i]!.label.toLowerCase().startsWith(k));
      if (hit !== undefined) setActive(hit);
    }
  };

  return (
    <>
      <button
        ref={trigger}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={state === "open"}
        aria-controls={state === "closed" ? undefined : `${id}-list`}
        aria-activedescendant={state === "open" && active >= 0 ? `${id}-${active}` : undefined}
        className={clsx("eth-select", `eth-select--${size}`, state === "open" && "eth-select--open", className)}
        disabled={disabled}
        onClick={() => (state === "open" ? close() : open())}
        onKeyDown={onKeyDown}
        {...rest}
      >
        <span className={clsx("eth-select__value", !current && "eth-select__value--placeholder")}>
          {current?.label ?? placeholder ?? ""}
        </span>
        <ChevronDown className="eth-select__chevron" aria-hidden />
      </button>
      {state !== "closed" && (
        <SelectList
          id={`${id}-list`}
          label={rest["aria-label"]}
          anchor={trigger}
          options={options}
          value={value}
          active={active}
          closing={state === "closing"}
          optionId={(i) => `${id}-${i}`}
          onActive={setActive}
          onPick={pick}
          onDismiss={() => close(false)}
          onClosed={() => setState((s) => (s === "closing" ? "closed" : s))}
        />
      )}
    </>
  );
}

interface SelectListProps<V extends string> {
  id: string;
  label: string | undefined;
  anchor: React.RefObject<HTMLButtonElement | null>;
  options: ReadonlyArray<SelectOption<V>>;
  value: V;
  active: number;
  closing: boolean;
  optionId: (i: number) => string;
  onActive: (i: number) => void;
  onPick: (i: number) => void;
  onDismiss: () => void;
  onClosed: () => void;
}

function SelectList<V extends string>(p: SelectListProps<V>) {
  const ref = useRef<HTMLDivElement>(null);
  const { anchor, closing, onDismiss, onClosed } = p;

  // Outside press / window blur / resize closes (scrolling the list itself doesn't).
  useEffect(() => {
    if (closing) return;
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!ref.current?.contains(t) && !anchor.current?.contains(t)) onDismiss();
    };
    document.addEventListener("pointerdown", onDown, true);
    window.addEventListener("blur", onDismiss);
    window.addEventListener("resize", onDismiss);
    return () => {
      document.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("blur", onDismiss);
      window.removeEventListener("resize", onDismiss);
    };
  }, [anchor, closing, onDismiss]);

  // Closing: unmount after the exit animation (or a timer where there is no animation).
  useEffect(() => {
    if (!closing) return;
    const t = setTimeout(onClosed, CLOSE_MS);
    return () => clearTimeout(t);
  }, [closing, onClosed]);

  // Under the trigger (above it when there's no room), at least as wide as it.
  useLayoutEffect(() => {
    const el = ref.current;
    const a = anchor.current?.getBoundingClientRect();
    if (!el || !a) return;
    const { width, height } = el.getBoundingClientRect();
    const w = Math.max(width, a.width);
    const below = a.bottom + height <= window.innerHeight - VIEWPORT_MARGIN || a.top - height < VIEWPORT_MARGIN;
    const left = Math.max(VIEWPORT_MARGIN, Math.min(a.left, window.innerWidth - w - VIEWPORT_MARGIN));
    el.style.minWidth = `${a.width}px`;
    el.style.left = `${left}px`;
    el.style.top = `${below ? a.bottom : a.top - height}px`;
    el.style.setProperty("--context-menu-dir", below ? "1" : "-1");
    el.style.transformOrigin = below ? "top left" : "bottom left";
    el.style.visibility = "visible";
  }, [anchor]);

  // Keep the active option in view.
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>(`[data-index="${p.active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [p.active]);

  return createPortal(
    <div
      ref={ref}
      id={p.id}
      role="listbox"
      aria-label={p.label}
      className={clsx("eth-popover", "eth-popover--menu", "eth-select-list", closing ? "eth-select-list--closing" : "eth-popover--context")}
      style={{ visibility: "hidden" }}
      onAnimationEnd={() => closing && onClosed()}
      onPointerDown={(e) => {
        // Keep focus on the trigger, and don't let a Popover around the trigger (the list is
        // portaled outside it) take this press for an outside click and close.
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      {p.options.map((o, i) => {
        const heading = o.group !== undefined && o.group !== p.options[i - 1]?.group ? o.group : null;
        return (
          <div key={o.value} role="presentation">
            {heading !== null && (
              <div className="eth-select-list__group" role="presentation">
                {heading}
              </div>
            )}
            <div
              id={p.optionId(i)}
              data-index={i}
              data-value={o.value}
              role="option"
              aria-selected={o.value === p.value}
              aria-disabled={o.disabled || undefined}
              className={clsx(
                "eth-menu__item",
                "eth-select-list__option",
                i === p.active && "eth-select-list__option--active",
                o.disabled && "eth-select-list__option--disabled",
              )}
              onPointerEnter={() => !o.disabled && p.onActive(i)}
              onClick={() => p.onPick(i)}
            >
              <span className="eth-select-list__check" aria-hidden>
                {o.value === p.value && <Check />}
              </span>
              {o.icon && (
                <span className="eth-select-list__icon" aria-hidden>
                  {o.icon}
                </span>
              )}
              <span className="eth-menu__label">{o.label}</span>
            </div>
          </div>
        );
      })}
    </div>,
    document.body,
  );
}
