/**
 * Window-level drags that end with their component.
 *
 * A drag helper listens on `window` until release. If the view that started it unmounts
 * mid-drag (a collaborator or an undo removes the lane, the clip or the note), its
 * listeners would keep firing against a dead view: measuring a null ref throws on every
 * move, and edits keep going to deleted items. Drag helpers return their cancel (the same
 * as a release: it removes the listeners and closes the undo gesture), and
 * `useScopedDrag(helper)` cancels every drag still running when the component unmounts.
 */

import { useCallback, useLayoutEffect, useState } from "react";

/**
 * Ends a running drag like a release: removes its window listeners and closes its gesture.
 * Idempotent; `settled` turns true once the drag ended (released or cancelled).
 */
export type DragCancel = (() => void) & { readonly settled: boolean };

/**
 * Wraps a drag's end so it runs once, whichever comes first: the release or the unmount.
 * Drag helpers call it on release and return it as their cancel.
 */
export function dragEnd(fn: () => void): DragCancel {
  const end = Object.assign(
    () => {
      if (end.settled) return;
      end.settled = true;
      fn();
    },
    { settled: false },
  );
  return end;
}

/** `register(cancel)`: the drag is cancelled if the component unmounts before it ends. */
export function useDragScope(): (cancel: DragCancel) => DragCancel {
  const [drags] = useState(() => new Set<DragCancel>());
  // A layout effect: its cleanup runs in the commit that removes the view, so no pointer
  // event reaches a drag of an unmounted view.
  useLayoutEffect(
    () => () => {
      const running = [...drags];
      drags.clear();
      for (const cancel of running) cancel();
    },
    [drags],
  );
  return useCallback(
    (cancel: DragCancel) => {
      // Forget the drags that already ended (the set only holds running ones).
      for (const c of drags) if (c.settled) drags.delete(c);
      drags.add(cancel);
      return cancel;
    },
    [drags],
  );
}

/** `helper` (a drag helper returning its cancel), cancelled if the component unmounts mid-drag. */
export function useScopedDrag<A extends unknown[]>(helper: (...args: A) => DragCancel): (...args: A) => DragCancel {
  const register = useDragScope();
  return (...args: A) => register(helper(...args));
}
