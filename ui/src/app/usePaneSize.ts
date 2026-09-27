import { useState } from "react";

/**
 * Pane size persisted per key in localStorage. Storage may be unavailable (private mode,
 * tests), in which case the default is used and nothing is saved.
 */
export function usePaneSize(key: string, initial: number): [number, (px: number) => void] {
  const [size, setSize] = useState(() => {
    try {
      const v = Number(localStorage.getItem(`eth.pane.${key}`));
      return Number.isFinite(v) && v > 0 ? v : initial;
    } catch {
      return initial;
    }
  });
  const set = (px: number) => {
    setSize(px);
    try {
      localStorage.setItem(`eth.pane.${key}`, String(Math.round(px)));
    } catch {
      /* not persisted */
    }
  };
  return [size, set];
}
