/**
 * App-wide tooltips for `title` attributes. Desktop webviews (Tauri on macOS) never show the
 * native `title` tooltip, so the app would have none: this host shows every `title` as the
 * kit tooltip (`.eth-tooltip`, after the `--tooltip-delay`), above the hovered element (below
 * when it is near the top of the window). While shown, the element's `title` is held in
 * `data-eth-title` so the native tooltip doesn't double it, and put back when the pointer
 * leaves (assistive tech and tests keep reading `title`). A press, a key, a scroll or the
 * pointer leaving the window hides it. Mount once (app root).
 */
import clsx from "clsx";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

interface Tip {
  id: number;
  text: string;
  x: number;
  y: number;
  placement: "top" | "bottom";
}

/** Elements this close to the window top get the tooltip below them. */
const TOP_ROOM_PX = 48;
/** Room kept between a tooltip and the window's sides. */
const SIDE_ROOM_PX = 8;

export function TitleTooltipHost() {
  const [tip, setTip] = useState<Tip | null>(null);
  const el = useRef<HTMLSpanElement>(null);
  // Centered on the element, but never past the window's sides.
  useLayoutEffect(() => {
    const node = el.current;
    if (!node || !tip) return;
    const r = node.getBoundingClientRect();
    const shift = Math.max(0, SIDE_ROOM_PX - r.left) - Math.max(0, r.right - (window.innerWidth - SIDE_ROOM_PX));
    if (shift) node.style.left = `${tip.x + shift}px`;
  }, [tip]);
  useEffect(() => {
    let current: HTMLElement | null = null;
    let next = 1;
    const restore = () => {
      if (current && !current.hasAttribute("title") && current.dataset.ethTitle) current.setAttribute("title", current.dataset.ethTitle);
      current = null;
    };
    const hide = () => {
      restore();
      setTip(null);
    };
    const over = (e: PointerEvent) => {
      if (e.pointerType === "touch") return;
      const el = e.target instanceof Element ? e.target.closest<HTMLElement>("[title]") : null;
      if (el === current) return;
      restore();
      if (!el) return setTip(null);
      const text = el.getAttribute("title") ?? "";
      el.dataset.ethTitle = text;
      el.removeAttribute("title");
      current = el;
      if (!text.trim()) return setTip(null);
      const r = el.getBoundingClientRect();
      const below = r.top < TOP_ROOM_PX;
      setTip({ id: next++, text, x: r.left + r.width / 2, y: below ? r.bottom : r.top, placement: below ? "bottom" : "top" });
    };
    const out = (e: PointerEvent) => {
      if (!e.relatedTarget) hide();
    };
    window.addEventListener("pointerover", over, true);
    window.addEventListener("pointerout", out, true);
    window.addEventListener("pointerdown", hide, true);
    window.addEventListener("keydown", hide, true);
    window.addEventListener("scroll", hide, true);
    window.addEventListener("blur", hide);
    return () => {
      hide();
      window.removeEventListener("pointerover", over, true);
      window.removeEventListener("pointerout", out, true);
      window.removeEventListener("pointerdown", hide, true);
      window.removeEventListener("keydown", hide, true);
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("blur", hide);
    };
  }, []);
  if (!tip) return null;
  return createPortal(
    <span ref={el} key={tip.id} role="tooltip" className={clsx("eth-tooltip", `eth-tooltip--${tip.placement}`)} style={{ left: tip.x, top: tip.y }}>
      {tip.text}
    </span>,
    document.body,
  );
}
