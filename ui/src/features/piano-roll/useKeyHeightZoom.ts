import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { ScaleFollower } from "@/timeline";
import { DEFAULT_KEY_HEIGHT, MAX_KEY_HEIGHT, MIN_KEY_HEIGHT } from "./geometry";

const KEY_HEIGHT_KEY = "eth-piano-roll-key-height";
/** Saving waits for the zoom animation to settle. */
const SAVE_DELAY_MS = 250;

function loadKeyHeight(): number {
  try {
    const v = Number(localStorage.getItem(KEY_HEIGHT_KEY));
    return Number.isFinite(v) && v >= MIN_KEY_HEIGHT && v <= MAX_KEY_HEIGHT ? v : DEFAULT_KEY_HEIGHT;
  } catch {
    return DEFAULT_KEY_HEIGHT;
  }
}

function saveKeyHeight(keyH: number): void {
  try {
    localStorage.setItem(KEY_HEIGHT_KEY, String(keyH));
  } catch {
    // Storage unavailable: the zoom lasts for this session only.
  }
}

/**
 * Vertical zoom of the piano roll (cmd/ctrl + shift + wheel, via `useTimelineWheel`'s
 * `onVerticalZoom`): animates the key height, keeping the pitch under the pointer in place
 * on every frame. The key height is remembered across sessions (localStorage; this session
 * only where storage is unavailable). Returns the current key height, the zoom handler, and a
 * direct setter for drags (`set(keyH, keysAtPointer, pointerY)`: no animation, the content
 * `keysAtPointer` keys from the top stays at the pointer's y).
 */
export function useKeyHeightZoom(
  bodyRef: RefObject<HTMLElement | null>,
): [number, (factor: number, pointerY: number) => void, (keyH: number, keysAtPointer: number, pointerY: number) => void] {
  const [keyH, setKeyH] = useState(loadKeyHeight);
  useEffect(() => {
    if (keyH === loadKeyHeight()) return;
    const t = setTimeout(() => saveKeyHeight(keyH), SAVE_DELAY_MS);
    return () => clearTimeout(t);
  }, [keyH]);
  const keyHRef = useRef(keyH);
  /** Content position under the pointer, in keys from the top, and the pointer's y. */
  const anchor = useRef<{ keys: number; pointerY: number } | null>(null);

  const follower = useMemo(() => new ScaleFollower((factor) => setKeyH((k) => Math.min(MAX_KEY_HEIGHT, Math.max(MIN_KEY_HEIGHT, k * factor)))), []);
  useEffect(() => () => follower.cancel(), [follower]);

  useLayoutEffect(() => {
    if (keyHRef.current === keyH) return;
    keyHRef.current = keyH;
    const a = anchor.current;
    const el = bodyRef.current;
    if (a && el) el.scrollTop = a.keys * keyH - a.pointerY;
  }, [keyH, bodyRef]);

  const onZoom = useCallback(
    (factor: number, pointerY: number) => {
      const el = bodyRef.current;
      const k = keyHRef.current;
      if (el) anchor.current = { keys: (el.scrollTop + pointerY) / k, pointerY };
      follower.push(factor, { min: MIN_KEY_HEIGHT / k, max: MAX_KEY_HEIGHT / k });
    },
    [follower, bodyRef],
  );

  const setDirect = useCallback(
    (k: number, keysAtPointer: number, pointerY: number) => {
      follower.cancel();
      anchor.current = { keys: keysAtPointer, pointerY };
      setKeyH(Math.min(MAX_KEY_HEIGHT, Math.max(MIN_KEY_HEIGHT, k)));
    },
    [follower],
  );

  return [keyH, onZoom, setDirect];
}
