import { useEffect, useLayoutEffect, useMemo, useRef, type KeyboardEvent } from "react";
import { useContextMenuStore, type OpenMenu } from "./contextMenuStore";
import { MenuList, type MenuEntry } from "./overlays";

/** Distance kept from the window edges. */
const VIEWPORT_MARGIN = 4;

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.tagName === "TEXTAREA") return true;
  return target instanceof HTMLInputElement && !["button", "checkbox", "radio", "range", "color", "file"].includes(target.type);
}

/**
 * Renders the open context menu (the kit's `MenuList` in a popover) and suppresses the
 * browser's own menu app-wide, except in text fields. Mounted once by the app shell.
 */
export function ContextMenuHost() {
  const menu = useContextMenuStore((s) => s.menu);

  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      if (!isTextEntry(e.target)) e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);

  return menu ? <ContextMenu key={menu.id} menu={menu} /> : null;
}

function ContextMenu({ menu }: { menu: OpenMenu }) {
  const close = useContextMenuStore((s) => s.close);
  const ref = useRef<HTMLDivElement>(null);
  const items = useMemo<MenuEntry[]>(
    () =>
      menu.items.map((it, i) =>
        it === "separator" ? { separator: true, id: `s${i}` } : { id: `i${i}`, ...it },
      ),
    [menu.items],
  );

  useEffect(() => {
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("wheel", close, { passive: true });
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("wheel", close);
    };
  }, [close]);

  // Keep the menu inside the window (flip up near the bottom).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const x = Math.max(VIEWPORT_MARGIN, Math.min(menu.x, window.innerWidth - width - VIEWPORT_MARGIN));
    const y = menu.y + height > window.innerHeight - VIEWPORT_MARGIN ? Math.max(VIEWPORT_MARGIN, menu.y - height) : menu.y;
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    // Grow out of the click point; fly downward, or upward when flipped above it.
    el.style.transformOrigin = `${menu.x - x}px ${menu.y - y}px`;
    el.style.setProperty("--context-menu-dir", y < menu.y ? "-1" : "1");
    el.style.visibility = "visible";
  }, [menu]);

  const onKeyDown = (e: KeyboardEvent) => {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };

  return (
    <div
      ref={ref}
      className="eth-popover eth-popover--menu eth-popover--context"
      style={{ left: menu.x, top: menu.y, visibility: "hidden" }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      <MenuList items={items} close={close} />
    </div>
  );
}
