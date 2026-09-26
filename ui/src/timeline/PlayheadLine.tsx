import { useRef, type CSSProperties } from "react";
import { usePlayheadPosition, type PlayheadMapping } from "./playhead";
import type { TimelineViewStore } from "./viewStore";
import "./timeline.css";

export interface PlayheadLineProps {
  view: TimelineViewStore;
  mapping?: PlayheadMapping;
  className?: string;
  style?: CSSProperties;
}

/** A full-height vertical playhead line; place it in a `position: relative` container. */
export function PlayheadLine({ view, mapping, className, style }: PlayheadLineProps) {
  const ref = useRef<HTMLDivElement>(null);
  usePlayheadPosition(ref, view, mapping);
  return (
    <div
      ref={ref}
      className={className ? `eth-playhead-line ${className}` : "eth-playhead-line"}
      style={style}
      aria-hidden
      data-testid="playhead-line"
    />
  );
}
