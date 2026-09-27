import clsx from "clsx";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useContextMenuStore, type MenuItem, type OpenMenu } from "./contextMenuStore";

const VIEWPORT_MARGIN = 4;

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable || target.tagName === "TEXTAREA") return true;
  return target instanceof HTMLInputElement && !["button", "checkbox", "radio", "range", "color", "file"].includes(target.type);
}

/** Renders the open context menu and suppresses the browser's own menu app-wide. */
export function ContextMenuHost() {
  const menu = useContextMenuStore((s) => s.menu);

  // Swallow the browser's menu everywhere except text fields (copy/paste).
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      if (!isTextEntry(e.target)) e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);

  return menu ? <Menu key={menu.id} menu={menu} /> : null;
}

function Menu({ menu }: { menu: OpenMenu }) {
  const close = useContextMenuStore((s) => s.close);
  const ref = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(-1);
  const { items } = menu;

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

  // Keep the menu inside the window (flip up near the bottom), then focus it for the keyboard.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const x = Math.max(VIEWPORT_MARGIN, Math.min(menu.x, window.innerWidth - width - VIEWPORT_MARGIN));
    const y = menu.y + height > window.innerHeight - VIEWPORT_MARGIN ? Math.max(VIEWPORT_MARGIN, menu.y - height) : menu.y;
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.style.visibility = "visible";
    el.focus({ preventScroll: true });
  }, [menu]);

  const enabled = items.flatMap((it, i) => (it !== "separator" && !it.disabled ? [i] : []));

  const choose = (item: MenuItem) => {
    close();
    item.onSelect();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!enabled.length) return;
      const at = enabled.indexOf(active);
      const next = e.key === "ArrowDown" ? (at + 1) % enabled.length : (at <= 0 ? enabled.length : at) - 1;
      setActive(enabled[next]!);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const item = items[active];
      if (item && item !== "separator" && !item.disabled) choose(item);
    }
  };

  return (
    <div
      ref={ref}
      className="eth-menu"
      role="menu"
      tabIndex={-1}
      style={{ left: menu.x, top: menu.y, visibility: "hidden" }}
      onKeyDown={onKeyDown}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} className="eth-menu__separator" role="separator" />
        ) : (
          <button
            key={i}
            type="button"
            role="menuitem"
            className={clsx("eth-menu__item", item.danger && "eth-menu__item--danger", i === active && "eth-menu__item--active")}
            disabled={item.disabled}
            onPointerEnter={() => !item.disabled && setActive(i)}
            onPointerLeave={() => setActive(-1)}
            onClick={() => choose(item)}
          >
            <span className="eth-menu__label">{item.label}</span>
            {item.shortcut && <span className="eth-menu__shortcut" aria-hidden>{item.shortcut}</span>}
          </button>
        ),
      )}
    </div>
  );
}
