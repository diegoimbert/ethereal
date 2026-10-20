/** The system back (button or gesture) closes the open sheet or panel (the design's §7). */
import { useEffect, useRef } from "react";

let nextEntry = 1;

/**
 * While `open`, a history entry (same URL) stands for the sheet: the system back closes it
 * instead of leaving the project. Closing it otherwise takes the entry back off.
 */
export function useBackToClose(open: boolean, onClose: () => void): void {
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });
  useEffect(() => {
    if (!open) return;
    const entry = nextEntry++;
    history.pushState({ ...(history.state as object | null), ethSheet: entry }, "", location.href);
    let popped = false;
    const onPop = () => {
      if ((history.state as { ethSheet?: number } | null)?.ethSheet === entry) return;
      popped = true;
      close.current();
    };
    window.addEventListener("popstate", onPop);
    return () => {
      window.removeEventListener("popstate", onPop);
      if (!popped && (history.state as { ethSheet?: number } | null)?.ethSheet === entry) history.back();
    };
  }, [open]);
}
