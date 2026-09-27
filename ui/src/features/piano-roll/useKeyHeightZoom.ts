import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { ScaleFollower } from "@/timeline";
import { DEFAULT_KEY_HEIGHT, MAX_KEY_HEIGHT, MIN_KEY_HEIGHT } from "./geometry";

/**
 * Vertical zoom of the piano roll (cmd/ctrl + shift + wheel, via `useTimelineWheel`'s
 * `onVerticalZoom`): animates the key height, keeping the pitch under the pointer in place
 * on every frame. Returns the current key height and the zoom handler.
 */
export function useKeyHeightZoom(bodyRef: RefObject<HTMLElement | null>): [number, (factor: number, pointerY: number) => void] {
  const [keyH, setKeyH] = useState(DEFAULT_KEY_HEIGHT);
  const keyHRef = useRef(keyH);
  /** Content position under the pointer, in keys from the top, and the pointer's y. */
  const anchor = useRef<{ keys: number; pointerY: number } | null>(null);

  const follower = useMemo(
    () =>
      new ScaleFollower((factor) =>
        setKeyH((k) => Math.min(MAX_KEY_HEIGHT, Math.max(MIN_KEY_HEIGHT, k * factor))),
      ),
    [],
  );
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

  return [keyH, onZoom];
}
