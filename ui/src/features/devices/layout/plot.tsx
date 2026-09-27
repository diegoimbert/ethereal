/**
 * Shared plumbing of the graphical widgets: an SVG plot sized to its (token-sized) box, and
 * pointer drags in plot coordinates wrapped in one undo gesture.
 */

import clsx from "clsx";
import { useRef, type PointerEvent, type ReactNode } from "react";
import { useBoxSize } from "./useBoxSize";

export interface PlotDrag {
  /** Pointer down at plot coords (px). Return false to ignore this press. */
  start(x: number, y: number, e: PointerEvent<SVGSVGElement>): boolean | void;
  move(x: number, y: number, e: PointerEvent<SVGSVGElement>): void;
  end?(): void;
}

export interface PlotProps {
  className?: string;
  /** Accessible description of the graphic. */
  label: string;
  drag?: PlotDrag;
  /** Gesture hooks around a drag (one undo step). */
  begin?(): void;
  end?(): void;
  onDoubleClick?(): void;
  children(size: { w: number; h: number }): ReactNode;
  testId?: string;
}

/** An SVG plot filling its CSS box; children draw in pixel coordinates. */
export function Plot({ className, label, drag, begin, end, onDoubleClick, children, testId }: PlotProps) {
  const [ref, size] = useBoxSize<HTMLDivElement>();
  const active = useRef(false);
  const local = (e: PointerEvent<SVGSVGElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top] as const;
  };
  const stop = () => {
    if (!active.current) return;
    active.current = false;
    drag?.end?.();
    end?.();
  };
  return (
    <div ref={ref} className={clsx("eth-plot", drag && "eth-plot--interactive", className)} data-testid={testId}>
      <svg
        className="eth-plot__svg"
        width={size.w}
        height={size.h}
        viewBox={`0 0 ${size.w} ${size.h}`}
        role="img"
        aria-label={label}
        onPointerDown={
          drag
            ? (e) => {
                if (e.button !== 0) return;
                const [x, y] = local(e);
                // Open the gesture first: a press that sets values (pads, steps) is part of it.
                begin?.();
                if (drag.start(x, y, e) === false) {
                  end?.();
                  return;
                }
                e.currentTarget.setPointerCapture?.(e.pointerId);
                active.current = true;
                e.preventDefault();
              }
            : undefined
        }
        onPointerMove={
          drag
            ? (e) => {
                if (!active.current) return;
                const [x, y] = local(e);
                drag.move(x, y, e);
              }
            : undefined
        }
        onPointerUp={drag ? stop : undefined}
        onPointerCancel={drag ? stop : undefined}
        onDoubleClick={onDoubleClick}
      >
        {children(size)}
      </svg>
    </div>
  );
}

/** Horizontal grid lines at fractions of the height (0 = top). */
export function GridLines({ w, h, ys, xs = [] }: { w: number; h: number; ys: ReadonlyArray<number>; xs?: ReadonlyArray<number> }) {
  return (
    <g className="eth-plot__grid">
      {ys.map((y, i) => (
        <line key={`y${i}`} x1={0} x2={w} y1={y * h} y2={y * h} />
      ))}
      {xs.map((x, i) => (
        <line key={`x${i}`} x1={x * w} x2={x * w} y1={0} y2={h} />
      ))}
    </g>
  );
}
