import clsx from "clsx";
import { Check, ChevronDown } from "lucide-react";
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type ButtonHTMLAttributes, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { size as sizeTokens } from "../theme/tokens";
import { clampToViewport, VIEWPORT_MARGIN } from "./placement";
import { useDisabled } from "./readOnly";
import type { Size } from "./variants";
import { VirtualList } from "./VirtualList";
import { foldForSearch, useT } from "@/i18n";

export interface SelectOption<V extends string> {
  value: V;
  label: string;
  disabled?: boolean;
  /** Options with the same group are listed under that heading (in first-seen order). */
  group?: string;
  /** Optional leading icon in the list. */
  icon?: ReactNode;
  /** Marks the option for the search's "Active" filter (see `activeFilter`). */
  active?: boolean;
  /** The label is user content (a track, device, file name…): keeps its case (`eth-user-text`). */
  userText?: boolean;
}

export interface SelectProps<V extends string> extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "value" | "onChange" | "children"> {
  options: ReadonlyArray<SelectOption<V>>;
  /** The selected value; one not among the options shows `placeholder`. */
  value: V;
  onChange: (value: V) => void;
  size?: Size;
  /** Shown when `value` matches no option (e.g. "+ Show parameter…"). */
  placeholder?: string;
  /**
   * A search field above a windowed list (only the visible options are mounted). Default:
   * on above `SEARCH_OVER` options (e.g. a plugin's thousands of parameters).
   */
  searchable?: boolean;
  /**
   * An "Active" toggle right of the search field that limits the list to options marked
   * `active`. `value` is the stored preference: it only applies while some option is
   * active; otherwise the full list shows and the toggle is off and disabled.
   */
  activeFilter?: { value: boolean; onChange: (value: boolean) => void };
}

/** Above this many options a Select is searchable and windowed by default. */
export const SEARCH_OVER = 50;
/** Must match the close animation (`--select-exit-duration`); unmounts if no animationend. */
const CLOSE_MS = 120;
/** Row height of the windowed list (= `--menu-item-height`, the `rowHeight` size token). */
const ROW_PX = parseFloat(sizeTokens.rowHeight);

/** Does `o` match a search (`q` folded: case and accents ignored, on its label or group)? */
function matches(o: SelectOption<string>, q: string): boolean {
  return foldForSearch(o.label).includes(q) || (o.group !== undefined && foldForSearch(o.group).includes(q));
}

/**
 * Drop-down select: a button showing the current option and our own animated list (not the
 * system one). Keyboard: ↑/↓ or Enter/Space opens; ↑/↓, Home/End and typing a letter move;
 * Enter/Space picks; Escape or Tab closes (in the app, Space is the transport's whenever a
 * project is open: `transport-bar/keys.ts`; it reaches here only without one). ARIA: a
 * `combobox` button controlling a `listbox`.
 *
 * Long lists (`searchable`) get a search field (focused on open; typing filters, ↑/↓ and
 * Enter work from it) and a windowed list that mounts only the visible options.
 */
export function Select<V extends string>({
  options,
  value,
  onChange,
  size = "md",
  placeholder,
  className,
  disabled: disabledProp,
  searchable = options.length > SEARCH_OVER,
  activeFilter,
  ...rest
}: SelectProps<V>) {
  const disabled = useDisabled(disabledProp);
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const [state, setState] = useState<"closed" | "open" | "closing">("closed");
  const [active, setActive] = useState(-1);
  const [query, setQuery] = useState("");
  const current = options.find((o) => o.value === value);
  const q = searchable ? foldForSearch(query.trim()) : "";
  /** Indices of the options listed (all, or the search matches). */
  const hasActive = useMemo(() => options.some((o) => o.active), [options]);
  const activeOnly = searchable && !!activeFilter?.value && hasActive;
  const shown = useMemo(() => options.flatMap((o, i) => ((!activeOnly || o.active) && (!q || matches(o, q)) ? [i] : [])), [options, q, activeOnly]);
  const enabled = shown.filter((i) => !options[i]!.disabled);

  const open = () => {
    if (disabled) return;
    setQuery("");
    setActive(
      Math.max(
        0,
        options.findIndex((o) => o.value === value),
      ),
    );
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
  const search = (text: string) => {
    setQuery(text);
    const folded = foldForSearch(text.trim());
    const first = options.findIndex((o) => !o.disabled && (!activeOnly || o.active) && (!folded || matches(o, folded)));
    setActive(first);
  };

  const onKeyDown = (e: KeyboardEvent, fromSearch = false) => {
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
      close(e.key === "Escape" || fromSearch);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => move(a, e.key === "ArrowDown" ? 1 : -1));
    } else if (e.key === "Home" || e.key === "End") {
      if (fromSearch) return; // caret moves in the search field
      e.preventDefault();
      setActive(e.key === "Home" ? (enabled[0] ?? -1) : (enabled[enabled.length - 1] ?? -1));
    } else if (e.key === "Enter" || (e.key === " " && !fromSearch)) {
      e.preventDefault();
      pick(active);
    } else if (!fromSearch && e.key.length === 1 && /\S/.test(e.key)) {
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
        onKeyDown={(e) => onKeyDown(e)}
        {...rest}
      >
        <span className={clsx("eth-select__value", !current && "eth-select__value--placeholder", current?.userText && "eth-user-text")}>
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
          shown={shown}
          search={searchable ? { query, onQuery: search, onKeyDown: (e) => onKeyDown(e, true) } : null}
          activeToggle={searchable && activeFilter ? { on: activeOnly, available: hasActive, onChange: activeFilter.onChange } : null}
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
  /** Indices of the options to list. */
  shown: ReadonlyArray<number>;
  /** Searchable (windowed) list, or `null` for the plain one. */
  search: { query: string; onQuery: (q: string) => void; onKeyDown: (e: KeyboardEvent) => void } | null;
  /** The "Active" toggle (`on`: filtering; `available`: some option is active). */
  activeToggle: { on: boolean; available: boolean; onChange: (value: boolean) => void } | null;
  value: V;
  active: number;
  closing: boolean;
  optionId: (i: number) => string;
  onActive: (i: number) => void;
  onPick: (i: number) => void;
  onDismiss: () => void;
  onClosed: () => void;
}

type Row = { group: string } | { index: number };

function SelectList<V extends string>(p: SelectListProps<V>) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const { anchor, closing, onDismiss, onClosed, search } = p;
  const searchRef = useRef<HTMLInputElement>(null);

  // Outside press / window blur / resize closes (scrolling the list itself doesn't).
  useEffect(() => {
    if (closing) return;
    const onDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (!ref.current?.contains(target) && !anchor.current?.contains(target)) onDismiss();
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
    const left = clampToViewport(a.left, w, window.innerWidth);
    el.style.minWidth = `${a.width}px`;
    el.style.left = `${left}px`;
    el.style.top = `${below ? a.bottom : a.top - height}px`;
    el.style.setProperty("--context-menu-dir", below ? "1" : "-1");
    el.style.transformOrigin = below ? "top left" : "bottom left";
    el.style.visibility = "visible";
  }, [anchor]);

  // The search field takes the keyboard when the list opens.
  const searching = search !== null;
  useEffect(() => {
    if (searching) searchRef.current?.focus({ preventScroll: true });
  }, [searching]);

  // Keep the active option in view (the windowed list scrolls itself).
  useEffect(() => {
    if (searching) return;
    ref.current?.querySelector<HTMLElement>(`[data-index="${p.active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [p.active, searching]);

  /** Group headings and options, in list order. */
  const rows = useMemo(() => {
    const out: Row[] = [];
    let prev: string | undefined;
    for (const i of p.shown) {
      const g = p.options[i]!.group;
      if (g !== undefined && g !== prev) out.push({ group: g });
      prev = g;
      out.push({ index: i });
    }
    return out;
  }, [p.options, p.shown]);

  const option = (i: number) => {
    const o = p.options[i]!;
    return (
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
        <span className={clsx("eth-menu__label", o.userText && "eth-user-text")} title={searching ? o.label : undefined}>
          {o.label}
        </span>
      </div>
    );
  };
  const heading = (g: string) => (
    <div className="eth-select-list__group" role="presentation">
      {g}
    </div>
  );

  const activeRow = searching ? rows.findIndex((r) => "index" in r && r.index === p.active) : -1;

  return createPortal(
    <div
      ref={ref}
      id={searching ? undefined : p.id}
      role={searching ? "dialog" : "listbox"}
      aria-label={p.label}
      className={clsx(
        "eth-popover",
        "eth-popover--menu",
        "eth-select-list",
        searching && "eth-select-list--search",
        closing ? "eth-select-list--closing" : "eth-popover--context",
      )}
      style={{ visibility: "hidden" }}
      onAnimationEnd={() => closing && onClosed()}
      onPointerDown={(e) => {
        // Don't let a Popover around the trigger (the list is portaled outside it) take this
        // press for an outside click and close.
        e.stopPropagation();
        // Keep focus on the trigger (or in the search field), except to click into that field.
        if (e.target !== searchRef.current) e.preventDefault();
      }}
    >
      {search ? (
        <>
          <div className="eth-select-list__searchbar">
            <input
              ref={searchRef}
              type="search"
              className="eth-input eth-input--sm eth-select-list__search"
              placeholder={t("kit.select.search_placeholder")}
              aria-label={p.label ? t("kit.select.search_named", { name: p.label }) : t("kit.select.search")}
              aria-controls={p.id}
              aria-activedescendant={p.active >= 0 ? p.optionId(p.active) : undefined}
              value={search.query}
              onChange={(e) => search.onQuery(e.target.value)}
              onKeyDown={search.onKeyDown}
            />
            {p.activeToggle && (
              <button
                type="button"
                className={clsx("eth-select-list__active", p.activeToggle.on && "eth-select-list__active--on")}
                aria-pressed={p.activeToggle.on}
                aria-label={t("kit.select.active_only")}
                title={p.activeToggle.available ? t("kit.select.active_only") : t("kit.select.no_active")}
                disabled={!p.activeToggle.available}
                onClick={() => p.activeToggle!.onChange(!p.activeToggle!.on)}
              >
                {t("kit.select.active")}
              </button>
            )}
          </div>
          <VirtualList
            id={p.id}
            role="listbox"
            aria-label={p.label}
            className="eth-select-list__rows"
            count={rows.length}
            rowHeight={ROW_PX}
            activeIndex={activeRow}
            rowKey={(r) => {
              const row = rows[r]!;
              return "group" in row ? `g:${row.group}:${r}` : `o:${p.options[row.index]!.value}`;
            }}
            renderRow={(r) => {
              const row = rows[r]!;
              return "group" in row ? heading(row.group) : option(row.index);
            }}
            empty={<div className="eth-select-list__empty">{t("kit.select.no_matches")}</div>}
          />
        </>
      ) : (
        p.shown.map((i) => {
          const o = p.options[i]!;
          const g = o.group !== undefined && o.group !== p.options[i - 1]?.group ? o.group : null;
          return (
            <div key={o.value} role="presentation">
              {g !== null && heading(g)}
              {option(i)}
            </div>
          );
        })
      )}
    </div>,
    document.body,
  );
}
