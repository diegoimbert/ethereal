import clsx from "clsx";
import { useRef, type KeyboardEvent, type ReactNode } from "react";
import type { Size } from "./variants";

export interface TabItem<Id extends string> {
  id: Id;
  label: ReactNode;
  disabled?: boolean;
}

export interface TabsProps<Id extends string> {
  items: ReadonlyArray<TabItem<Id>>;
  value: Id;
  onChange: (id: Id) => void;
  /** Accessible name of the tablist. */
  label: string;
  size?: Size;
  className?: string;
}

/** Tab strip (`role="tablist"`). Arrow keys move between tabs. Content is the caller's job. */
export function Tabs<Id extends string>({ items, value, onChange, label, size = "sm", className }: TabsProps<Id>) {
  const ref = useRef<HTMLDivElement>(null);
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const dir = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!dir) return;
    const enabled = items.filter((i) => !i.disabled);
    const idx = enabled.findIndex((i) => i.id === value);
    const next = enabled[(idx + dir + enabled.length) % enabled.length];
    if (!next) return;
    e.preventDefault();
    onChange(next.id);
    ref.current?.querySelectorAll<HTMLElement>('[role="tab"]')[items.indexOf(next)]?.focus();
  };
  return (
    <div
      ref={ref}
      className={clsx("eth-tabs", `eth-tabs--${size}`, className)}
      role="tablist"
      aria-label={label}
      onKeyDown={onKeyDown}
    >
      {items.map((it) => {
        const selected = it.id === value;
        return (
          <button
            key={it.id}
            type="button"
            role="tab"
            className="eth-tabs__tab"
            aria-selected={selected}
            tabIndex={selected ? 0 : -1}
            disabled={it.disabled}
            onClick={() => onChange(it.id)}
          >
            {it.label}
          </button>
        );
      })}
    </div>
  );
}
