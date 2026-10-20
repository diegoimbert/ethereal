import type { ReactNode, SyntheticEvent } from "react";
import { useReadOnly } from "@/state/viewer";

/** Controls inside a read-only surface that still work (view settings: the loop brace, tabs). */
const ALLOWED = '[data-viewer-allow], [role="tab"]';
/** What a click activates. */
const CONTROL = 'button, [role="button"], [role="switch"], [role="menuitem"], [role="checkbox"], a[href]';

const allowed = (e: SyntheticEvent) => e.target instanceof Element && e.target.closest(ALLOWED) !== null;

/**
 * An editing surface (piano roll, automation lanes, device views, inspector, marker lane) that,
 * while the app shows a shared project read-only, edits nothing: its
 * drags, draws, double-clicks and drops never start, and its buttons are dimmed and inert.
 * Scrolling and the wheel still work, its kit value controls are disabled on their own
 * (`useDisabled`), and what is marked `data-viewer-allow` (view settings) keeps working.
 * Outside the viewer it renders its children as they are.
 */
export function ReadOnlySurface({ children }: { children: ReactNode }) {
  const readOnly = useReadOnly();
  if (!readOnly) return <>{children}</>;
  const stop = (e: SyntheticEvent) => {
    if (!allowed(e)) e.stopPropagation();
  };
  const block = (e: SyntheticEvent) => {
    if (allowed(e)) return;
    e.stopPropagation();
    e.preventDefault();
  };
  const click = (e: SyntheticEvent) => {
    if (e.target instanceof Element && e.target.closest(CONTROL)) block(e);
  };
  return (
    <div
      className="eth-read-only-surface"
      data-read-only
      onPointerDownCapture={stop}
      onMouseDownCapture={stop}
      onDoubleClickCapture={stop}
      onClickCapture={click}
      onKeyDownCapture={stop}
      onDragStartCapture={block}
      onDropCapture={block}
    >
      {children}
    </div>
  );
}
