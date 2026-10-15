import { useEffect, type KeyboardEvent, type RefObject } from "react";

/**
 * The keyboard model every menu shares (the kit `MenuList`, the context menu, the project
 * menu and its submenus).
 */

/** The focusable items of `menu` itself (a nested submenu's items excluded), in order. */
export function menuItems(menu: HTMLElement | null): HTMLElement[] {
  if (!menu) return [];
  return [...menu.querySelectorAll<HTMLElement>('[role="menuitem"]:not(:disabled)')].filter((el) => el.closest('[role="menu"]') === menu);
}

/**
 * ↑/↓ move focus through `menu`'s own items (wrapping), Home/End jump to the first/last.
 * Only from one of its items (or the menu itself), so a control inside a menu (a select)
 * keeps its arrows. Returns whether it handled the key.
 */
export function menuKeyDown(e: KeyboardEvent<HTMLElement>, menu: HTMLElement | null): boolean {
  if (!menu || !["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return false;
  const els = menuItems(menu);
  const target = e.target as HTMLElement;
  if (target !== menu && !els.includes(target)) return false;
  e.preventDefault();
  e.stopPropagation();
  const n = els.length;
  const i = els.indexOf(document.activeElement as HTMLElement);
  const down = e.key === "ArrowDown";
  const next = e.key === "Home" ? 0 : e.key === "End" ? n - 1 : i < 0 ? (down ? 0 : n - 1) : (i + (down ? 1 : -1) + n) % n;
  els[next]?.focus();
  return true;
}

/** Focus the first item of the menu `ref` once it mounts (every menu opens like this). */
export function useFocusFirstItem(ref: RefObject<HTMLElement | null>) {
  useEffect(() => {
    menuItems(ref.current)[0]?.focus();
  }, [ref]);
}

/**
 * A menu item's disabled state: plain `disabled`, or, with a `reason`, `aria-disabled` (still
 * focusable and hoverable) and the reason as its tooltip / accessible description. The click
 * handler must ignore an `aria-disabled` item itself.
 */
export function disabledProps(disabled: boolean | undefined, reason: string | undefined) {
  if (!disabled) return {};
  if (!reason) return { disabled: true };
  return { "aria-disabled": true, title: reason } as const;
}
