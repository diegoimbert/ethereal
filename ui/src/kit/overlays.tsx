import clsx from "clsx";
import {
  cloneElement,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactElement,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import type { StatusTone } from "./variants";

// ---- Popover --------------------------------------------------------------------------

export type Placement = "bottom-start" | "bottom-end" | "top-start" | "top-end";

export interface TriggerProps {
  onClick: () => void;
  "aria-expanded": boolean;
  "aria-haspopup": "menu" | "dialog";
}

export interface PopoverProps {
  /** Renders the trigger; spread the given props onto a button. */
  trigger: (props: TriggerProps) => ReactNode;
  children: ReactNode | ((close: () => void) => ReactNode);
  /** Controlled open state (omit for uncontrolled). */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  placement?: Placement;
  /** ARIA role of the floating panel. */
  role?: string;
  "aria-label"?: string;
  className?: string;
  /** @internal popup kind announced by the trigger. */
  haspopup?: "menu" | "dialog";
}

/** Floating panel anchored to a trigger. Closes on outside pointer-down and Escape. */
export function Popover({
  trigger,
  children,
  open: openProp,
  onOpenChange,
  placement = "bottom-start",
  role = "dialog",
  className,
  haspopup = "dialog",
  ...aria
}: PopoverProps) {
  const [openState, setOpenState] = useState(false);
  const open = openProp ?? openState;
  const anchor = useRef<HTMLSpanElement>(null);
  const setOpen = (o: boolean) => {
    if (openProp === undefined) setOpenState(o);
    onOpenChange?.(o);
  };
  const close = () => setOpen(false);
  const closeRef = useRef(close);
  useEffect(() => {
    closeRef.current = close;
  });

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (anchor.current && !anchor.current.contains(e.target as Node)) closeRef.current();
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape" && open) {
      e.stopPropagation();
      close();
      anchor.current?.querySelector<HTMLElement>("[aria-expanded]")?.focus();
    }
  };

  return (
    <span ref={anchor} className="eth-popover-anchor" onKeyDown={onKeyDown}>
      {trigger({ onClick: () => setOpen(!open), "aria-expanded": open, "aria-haspopup": haspopup })}
      {open && (
        <div className={clsx("eth-popover", `eth-popover--${placement}`, className)} role={role} aria-label={aria["aria-label"]}>
          {typeof children === "function" ? children(close) : children}
        </div>
      )}
    </span>
  );
}

// ---- Menu -----------------------------------------------------------------------------

export type MenuEntry =
  | { id: string; label: ReactNode; onSelect: () => void; disabled?: boolean; danger?: boolean; shortcut?: string }
  | { separator: true; id: string };

export interface MenuProps {
  trigger: (props: TriggerProps) => ReactNode;
  items: ReadonlyArray<MenuEntry>;
  placement?: Placement;
  "aria-label"?: string;
}

function MenuList({ items, close, label }: { items: ReadonlyArray<MenuEntry>; close: () => void; label?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('[role="menuitem"]:not(:disabled)')?.focus();
  }, []);
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const els = [...(ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)') ?? [])];
    const i = els.indexOf(document.activeElement as HTMLElement);
    const n = els.length;
    els[(i + (e.key === "ArrowDown" ? 1 : -1) + n) % n]?.focus();
  };
  return (
    <div ref={ref} className="eth-menu" role="menu" aria-label={label} onKeyDown={onKeyDown}>
      {items.map((it) =>
        "separator" in it ? (
          <div key={it.id} className="eth-menu__separator" role="separator" />
        ) : (
          <button
            key={it.id}
            type="button"
            role="menuitem"
            className={clsx("eth-menu__item", it.danger && "eth-menu__item--danger")}
            disabled={it.disabled}
            onClick={() => {
              close();
              it.onSelect();
            }}
          >
            <span className="eth-menu__label">{it.label}</span>
            {it.shortcut && <span className="eth-menu__shortcut">{it.shortcut}</span>}
          </button>
        ),
      )}
    </div>
  );
}

/** Dropdown menu: a Popover holding `role="menu"` items (↑/↓ to move, Enter to pick, Esc to close). */
export function Menu({ trigger, items, placement, ...aria }: MenuProps) {
  return (
    <Popover trigger={trigger} placement={placement} role="presentation" haspopup="menu" className="eth-popover--menu">
      {(close) => <MenuList items={items} close={close} label={aria["aria-label"]} />}
    </Popover>
  );
}

// ---- Dialog ---------------------------------------------------------------------------

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children?: ReactNode;
  /** Right-aligned actions (buttons). */
  footer?: ReactNode;
  className?: string;
}

/** Modal dialog, rendered in a portal. Escape or a backdrop click closes it. */
export function Dialog({ open, onClose, title, children, footer, className }: DialogProps) {
  const titleId = useId();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const prev = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    return () => prev?.focus?.();
  }, [open]);
  if (!open) return null;
  return createPortal(
    <div
      className="eth-dialog-backdrop"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        className={clsx("eth-dialog", className)}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            onClose();
          }
        }}
      >
        <header className="eth-dialog__header" id={titleId}>
          {title}
        </header>
        <div className="eth-dialog__body">{children}</div>
        {footer !== undefined && <footer className="eth-dialog__footer">{footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}

// ---- Tooltip --------------------------------------------------------------------------

export interface TooltipProps {
  content: ReactNode;
  /** A single element (e.g. a button); receives `aria-describedby` while the tooltip shows. */
  children: ReactElement<{ "aria-describedby"?: string }>;
  placement?: "top" | "bottom";
}

/** Hover/focus tooltip. The show delay is the `--tooltip-delay` token. */
export function Tooltip({ content, children, placement = "top" }: TooltipProps) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <span
      className="eth-tooltip-anchor"
      onPointerEnter={() => setOpen(true)}
      onPointerLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)}
      onBlur={() => setOpen(false)}
    >
      {cloneElement(children, { "aria-describedby": open ? id : undefined })}
      {open && (
        <span id={id} role="tooltip" className={clsx("eth-tooltip", `eth-tooltip--${placement}`)}>
          {content}
        </span>
      )}
    </span>
  );
}

// ---- Badge ----------------------------------------------------------------------------

export interface BadgeProps {
  children: ReactNode;
  tone?: StatusTone;
  className?: string;
}

/** Small status label / count. */
export function Badge({ children, tone = "default", className }: BadgeProps) {
  return <span className={clsx("eth-badge", `eth-badge--${tone}`, className)}>{children}</span>;
}
