import clsx from "clsx";
import {
  cloneElement,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactElement,
  type RefObject,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { useT } from "@/i18n";
import { IconButton } from "./IconButton";
import { disabledProps, menuKeyDown, useFocusFirstItem } from "./menuKeyboard";
import { fitPlacement, type Fit, type Placement } from "./placement";
import type { StatusTone } from "./variants";

// ---- Anchoring ------------------------------------------------------------------------

/**
 * Viewport rect of `anchor` while `active`, kept fresh on scroll/resize. Floating layers
 * (Popover, Menu, Tooltip) render in a portal on <body> with `position: fixed` at this rect,
 * so panels with `overflow: hidden/auto` never clip them.
 */
function useAnchorRect(anchor: RefObject<HTMLElement | null>, active: boolean): DOMRect | null {
  const [rect, setRect] = useState<DOMRect | null>(null);
  useLayoutEffect(() => {
    if (!active) return;
    const update = () => {
      if (anchor.current) setRect(anchor.current.getBoundingClientRect());
    };
    update();
    window.addEventListener("scroll", update, true);
    window.addEventListener("resize", update);
    return () => {
      window.removeEventListener("scroll", update, true);
      window.removeEventListener("resize", update);
    };
  }, [anchor, active]);
  return active ? rect : null;
}

function placementStyle(placement: Placement, r: DOMRect | null, dx = 0): CSSProperties {
  if (!r) return {}; // first render only: the layout effect positions it before paint
  const [side, align] = placement.split("-") as ["top" | "bottom", "start" | "end"];
  return {
    ...(side === "bottom" ? { top: r.bottom } : { bottom: window.innerHeight - r.top }),
    ...(align === "start" ? { left: r.left + dx } : { right: window.innerWidth - r.right - dx }),
  };
}

// ---- Popover --------------------------------------------------------------------------

export type { Placement };

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

/** Unmount delay of a closing popover (its exit animation; see kit.css). */
const POPOVER_EXIT_MS = 120;

const ORIGIN: Record<Placement, string> = {
  "bottom-start": "top left",
  "bottom-end": "top right",
  "top-start": "bottom left",
  "top-end": "bottom right",
};

/**
 * Floating panel anchored to a trigger. Closes on outside pointer-down and Escape.
 * Animated like the context menu and the Select list: grows out of the trigger when it
 * opens, fades back when it closes (hidden from assistive tech while it does).
 */
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
  const panel = useRef<HTMLDivElement>(null);
  // Mounted while open and during the exit animation.
  const [shown, setShown] = useState(open);
  if (open && !shown) setShown(true);
  const closing = shown && !open;
  useEffect(() => {
    if (!closing) return;
    const t = setTimeout(() => setShown(false), POPOVER_EXIT_MS);
    return () => clearTimeout(t);
  }, [closing]);
  const rect = useAnchorRect(anchor, shown);
  // Measured once laid out (and when its content resizes): flipped / nudged into the window.
  const [fit, setFit] = useState<Fit | null>(null);
  useLayoutEffect(() => {
    const el = panel.current;
    if (!shown || !rect || !el) return;
    const measure = () => {
      // offset* ignore the open animation's scale.
      const h = el.style.maxHeight ? el.scrollHeight : el.offsetHeight;
      setFit(fitPlacement(placement, rect, el.offsetWidth, h, window.innerWidth, window.innerHeight));
    };
    measure();
    const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    ro?.observe(el);
    return () => ro?.disconnect();
  }, [shown, rect, placement]);
  const actual = fit?.placement ?? placement;
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
      const t = e.target as Node;
      if (!anchor.current?.contains(t) && !panel.current?.contains(t)) closeRef.current();
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
      {shown &&
        createPortal(
          // React events bubble through the portal to the anchor span (Escape handling).
          <div
            ref={panel}
            className={clsx("eth-popover", `eth-popover--${actual}`, closing ? "eth-popover--closing" : "eth-popover--enter", className)}
            style={{
              ...placementStyle(actual, rect, fit?.dx),
              ...(fit?.maxHeight !== undefined && { maxHeight: fit.maxHeight, overflowY: "auto" }),
              transformOrigin: ORIGIN[actual],
              ["--context-menu-dir" as string]: actual.startsWith("top") ? "-1" : "1",
            }}
            role={role}
            aria-label={aria["aria-label"]}
            aria-hidden={closing || undefined}
          >
            {typeof children === "function" ? children(close) : children}
          </div>,
          document.body,
        )}
    </span>
  );
}

// ---- Menu -----------------------------------------------------------------------------

export type MenuEntry =
  | {
      id: string;
      label: ReactNode;
      onSelect: () => void;
      disabled?: boolean;
      danger?: boolean;
      shortcut?: string;
      /**
       * Why the item is disabled: its tooltip and accessible description. A disabled item
       * with a reason stays focusable (`aria-disabled`), so keyboard users reach and hear it.
       */
      reason?: string;
      /** The label is user content (a track, device, file name…): keeps its case (`eth-user-text`). */
      userText?: boolean;
    }
  | { separator: true; id: string };

function MenuItemButton({ it, close }: { it: Extract<MenuEntry, { onSelect: () => void }>; close: () => void }) {
  return (
    <button
      type="button"
      role="menuitem"
      className={clsx("eth-menu__item", it.danger && "eth-menu__item--danger")}
      {...disabledProps(it.disabled, it.reason)}
      onClick={() => {
        if (it.disabled) return;
        close();
        it.onSelect();
      }}
    >
      <span className={clsx("eth-menu__label", it.userText && "eth-user-text")}>{it.label}</span>
      {it.shortcut && (
        <span className="eth-menu__shortcut" aria-hidden>
          {it.shortcut}
        </span>
      )}
    </button>
  );
}

export interface MenuProps {
  trigger: (props: TriggerProps) => ReactNode;
  items: ReadonlyArray<MenuEntry>;
  placement?: Placement;
  "aria-label"?: string;
}

/** The items of a menu (keyboard: ↑/↓ move focus); used by `Menu` and the context menu. */
export function MenuList({ items, close, label }: { items: ReadonlyArray<MenuEntry>; close: () => void; label?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useFocusFirstItem(ref);
  return (
    <div ref={ref} className="eth-menu" role="menu" aria-label={label} onKeyDown={(e) => menuKeyDown(e, ref.current)}>
      {items.map((it) =>
        "separator" in it ? <div key={it.id} className="eth-menu__separator" role="separator" /> : <MenuItemButton key={it.id} it={it} close={close} />,
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
  /**
   * A × in the header that calls `onClose`. Default: shown when there is no `footer` (a
   * dialog always has a visible way out, not only Escape or a backdrop click).
   */
  closeButton?: boolean;
  className?: string;
  /** `data-testid` of the dialog element. */
  testId?: string;
  /** A click on the backdrop closes it (default true). */
  backdropClose?: boolean;
}

/** Unmount delay of a closing dialog (its exit animation; see kit.css). */
const DIALOG_EXIT_MS = 150;

/** The open dialogs, the topmost last: Escape outside any dialog closes only the topmost. */
const openDialogs: HTMLElement[] = [];

const FOCUSABLE = [
  "a[href]",
  "button:not(:disabled)",
  "input:not(:disabled)",
  "select:not(:disabled)",
  "textarea:not(:disabled)",
  "[contenteditable='true']",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

/** The elements Tab reaches inside `root`, in order. */
function tabbables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.tabIndex >= 0 && !el.closest("[hidden], [inert]"));
}

/** Keep Tab inside `dialog`: from its last tabbable to the first, Shift+Tab the other way. */
function trapTab(e: KeyboardEvent<HTMLElement>, dialog: HTMLElement | null) {
  const active = document.activeElement as HTMLElement | null;
  // Focus in a layer of its own (a select's list, a context menu) tabs as it likes.
  if (!dialog || !active || !dialog.contains(active)) return;
  const els = tabbables(dialog);
  const first = els[0];
  const last = els.at(-1);
  let to: HTMLElement | undefined;
  if (!first || !last) to = dialog;
  else if (e.shiftKey && (active === first || active === dialog)) to = last;
  else if (!e.shiftKey && active === last) to = first;
  if (!to) return;
  e.preventDefault();
  to.focus();
}

/**
 * Modal dialog, rendered in a portal. Tab and Shift+Tab cycle inside it; Escape (wherever
 * focus is while it is the topmost dialog), a backdrop click or its × close it.
 */
export function Dialog({ open, onClose, title, children, footer, closeButton, className, testId, backdropClose = true }: DialogProps) {
  const t = useT();
  const titleId = useId();
  const ref = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  useEffect(() => {
    onCloseRef.current = onClose;
  });
  // Escape with focus outside the dialog (e.g. on <body> after a click on the backdrop's
  // blobs): the topmost dialog closes. Inside it, the dialog's own handler runs first.
  useEffect(() => {
    const el = ref.current;
    if (!open || !el) return;
    openDialogs.push(el);
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || openDialogs.at(-1) !== el) return;
      e.preventDefault();
      onCloseRef.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      openDialogs.splice(openDialogs.indexOf(el), 1);
    };
  }, [open]);
  // Mounted while open and during the exit animation, still showing what it last showed
  // (callers often render their content only while `open`).
  const [shown, setShown] = useState(open);
  const [last, setLast] = useState({ title, children, footer });
  if (open && !shown) setShown(true);
  if (open && (last.title !== title || last.children !== children || last.footer !== footer)) setLast({ title, children, footer });
  const closing = shown && !open;
  useEffect(() => {
    if (!closing) return;
    const t = setTimeout(() => setShown(false), DIALOG_EXIT_MS);
    return () => clearTimeout(t);
  }, [closing]);
  useEffect(() => {
    if (!open) return;
    const prev = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    return () => prev?.focus?.();
  }, [open]);
  if (!shown) return null;
  const view = open ? { title, children, footer } : last;
  return createPortal(
    <div
      className={clsx("eth-dialog-backdrop", closing && "eth-dialog-backdrop--closing")}
      aria-hidden={closing || undefined}
      onPointerDown={(e) => {
        if (!closing && backdropClose && e.target === e.currentTarget) onClose();
      }}
    >
      {/* Slow, very blurred color blobs drifting behind the dialog (decoration). */}
      <AmbientBlobs strength={0.75} />
      <div
        ref={ref}
        className={clsx("eth-dialog", closing && "eth-dialog--closing", className)}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        data-testid={testId}
        tabIndex={-1}
        onKeyDown={(e) => {
          if (e.key === "Escape" && !closing) {
            e.stopPropagation();
            onClose();
          }
          if (e.key === "Tab" && !e.defaultPrevented) trapTab(e, ref.current);
        }}
      >
        <header className="eth-dialog__header">
          <span id={titleId} className="eth-dialog__title">
            {view.title}
          </span>
          {(closeButton ?? view.footer === undefined) && (
            <IconButton className="eth-dialog__close" size="sm" tone="ghost" label={t("common.close")} icon={<X />} onClick={() => !closing && onClose()} />
          )}
        </header>
        <div className="eth-dialog__body">{view.children}</div>
        {view.footer !== undefined && <footer className="eth-dialog__footer">{view.footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}

// ---- Ambient blobs ---------------------------------------------------------------------

/**
 * Slow, very blurred color blobs drifting (decoration): fills its positioned parent, under
 * the content. `strength` scales their opacity (1: behind a modal).
 */
export function AmbientBlobs({ className, strength }: { className?: string; strength?: number }) {
  return (
    <div
      className={clsx("eth-ambient-blobs", className)}
      style={strength === undefined ? undefined : ({ "--ambient-strength": strength } as CSSProperties)}
      aria-hidden
    >
      <span />
      <span />
      <span />
      <span />
    </div>
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
  const anchor = useRef<HTMLSpanElement>(null);
  const r = useAnchorRect(anchor, open);
  const style: CSSProperties = r ? { left: r.left + r.width / 2, top: placement === "top" ? r.top : r.bottom } : {};
  return (
    <span
      ref={anchor}
      className="eth-tooltip-anchor"
      onPointerEnter={() => setOpen(true)}
      onPointerLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)}
      onBlur={() => setOpen(false)}
    >
      {cloneElement(children, { "aria-describedby": open ? id : undefined })}
      {open &&
        createPortal(
          <span id={id} role="tooltip" className={clsx("eth-tooltip", `eth-tooltip--${placement}`)} style={style}>
            {content}
          </span>,
          document.body,
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
