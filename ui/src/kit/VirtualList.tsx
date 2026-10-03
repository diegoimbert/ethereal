import clsx from "clsx";
import { useLayoutEffect, useRef, useState, type HTMLAttributes, type ReactNode } from "react";

/** Most rows a `VirtualList` ever mounts, whatever the viewport height. */
export const VIRTUAL_MAX_ROWS = 100;
/** Rows rendered above and below the visible window (smooth scrolling). */
const OVERSCAN = 6;
/** Viewport assumed before layout (and where there is none, e.g. jsdom). */
const FALLBACK_VIEWPORT_PX = 320;

export interface VirtualListProps extends Omit<HTMLAttributes<HTMLDivElement>, "children"> {
  /** Number of rows. */
  count: number;
  /** Fixed row height in CSS pixels (read it from a `size` token: `parseFloat(size.x)`). */
  rowHeight: number;
  /** Renders row `index`; only rows in (or near) the viewport are mounted. */
  renderRow: (index: number) => ReactNode;
  /** Stable React key of row `index` (default: the index). */
  rowKey?: (index: number) => string | number;
  /** Keep this row scrolled into view (keyboard navigation). */
  activeIndex?: number;
  /** Rendered instead of the rows when `count` is 0. */
  empty?: ReactNode;
}

/**
 * A windowed list of fixed-height rows: scrolls like the full list, mounts only the rows
 * in view (plus a small overscan, never more than `VIRTUAL_MAX_ROWS`). Smooth at tens of
 * thousands of rows. The scroll container is this element: give it a height (or a
 * max-height) in CSS.
 */
export function VirtualList({ count, rowHeight, renderRow, rowKey, activeIndex, empty, className, style, ...rest }: VirtualListProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(FALLBACK_VIEWPORT_PX);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setViewport(el.clientHeight || FALLBACK_VIEWPORT_PX);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // A shorter list (e.g. a narrower search) can leave the scroll position past its end.
  const maxTop = Math.max(0, count * rowHeight - viewport);
  const top = Math.min(scrollTop, maxTop);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || activeIndex === undefined || activeIndex < 0) return;
    const y = activeIndex * rowHeight;
    if (y < el.scrollTop) el.scrollTop = y;
    else if (y + rowHeight > el.scrollTop + el.clientHeight) el.scrollTop = y + rowHeight - (el.clientHeight || viewport);
    setScrollTop(el.scrollTop);
  }, [activeIndex, rowHeight, viewport]);

  const first = Math.max(0, Math.floor(top / rowHeight) - OVERSCAN);
  const visible = Math.ceil(viewport / rowHeight) + 2 * OVERSCAN;
  const last = Math.min(count, first + Math.min(visible, VIRTUAL_MAX_ROWS));
  const rows: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    rows.push(
      <div key={rowKey ? rowKey(i) : i} className="eth-vlist__row" role="presentation" style={{ top: i * rowHeight, height: rowHeight }}>
        {renderRow(i)}
      </div>,
    );
  }

  return (
    <div
      ref={ref}
      className={clsx("eth-vlist", className)}
      style={style}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      {...rest}
    >
      {count === 0 ? (
        empty
      ) : (
        <div className="eth-vlist__spacer" style={{ height: count * rowHeight }} role="presentation">
          {rows}
        </div>
      )}
    </div>
  );
}
