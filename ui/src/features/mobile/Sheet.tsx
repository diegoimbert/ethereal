/**
 * A bottom sheet (the design's §7): a panel sliding up over the lower part of the screen,
 * with a grab handle. Two heights ("medium" about half the screen, "tall" nearly all of it):
 * drag the handle up to grow it, down to shrink or close it; the scrim behind closes it; so do
 * Escape and the system back (button or gesture: opening pushes a history entry, same URL).
 * Like any modal it takes the focus while open (and gives it back on close): Escape then
 * reaches it even when the sheet was opened from the arrangement, whose own Escape
 * (deselect) would otherwise swallow it.
 * In phone landscape it is a full-screen panel (mobile.css).
 */
import { useEffect, useRef, useState, type PointerEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { IconButton } from "@/kit";
import { useBackToClose } from "./backToClose";
import { useT } from "@/i18n";

export type Detent = "medium" | "tall";

export interface SheetProps {
  open: boolean;
  onClose(): void;
  /** The dialog's accessible name. */
  label: string;
  title: ReactNode;
  detent?: Detent;
  children: ReactNode;
  testId?: string;
}

/** A drag of the handle past this (px) changes the height or closes. */
const SNAP_PX = 64;

export function Sheet({ open, onClose, label, title, detent: initial = "medium", children, testId }: SheetProps) {
  const t = useT();
  const [detent, setDetent] = useState<Detent>(initial);
  const [dragY, setDragY] = useState(0);
  const drag = useRef<{ y: number; id: number } | null>(null);
  const panel = useRef<HTMLElement>(null);
  useBackToClose(open, onClose);
  useEffect(() => {
    if (!open) return;
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    panel.current?.focus({ preventScroll: true });
    return () => {
      if (before?.isConnected) before.focus({ preventScroll: true });
    };
  }, [open]);
  // Each opening starts at its initial height.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setDetent(initial);
  }
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;

  const onDown = (e: PointerEvent<HTMLElement>) => {
    if (e.button !== 0 || (e.target instanceof Element && e.target.closest("button"))) return;
    drag.current = { y: e.clientY, id: e.pointerId };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };
  const onMove = (e: PointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    const dy = e.clientY - d.y;
    // Down follows the finger; up resists (the tall height is the limit).
    setDragY(dy > 0 ? dy : dy / 3);
  };
  const onUp = (e: PointerEvent<HTMLElement>) => {
    const d = drag.current;
    if (!d || d.id !== e.pointerId) return;
    drag.current = null;
    const dy = e.clientY - d.y;
    setDragY(0);
    if (dy > SNAP_PX) {
      if (detent === "tall") setDetent("medium");
      else onClose();
    } else if (dy < -SNAP_PX) setDetent("tall");
  };

  return createPortal(
    <div className="eth-sheet-layer" data-testid={testId}>
      <div className="eth-sheet__scrim" aria-hidden onClick={onClose} />
      <section
        ref={panel}
        tabIndex={-1}
        className="eth-sheet"
        role="dialog"
        aria-modal="true"
        aria-label={label}
        data-detent={detent}
        style={dragY !== 0 ? { transform: `translateY(${dragY}px)`, transition: "none" } : undefined}
      >
        <header className="eth-sheet__head" onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp}>
          <span className="eth-sheet__grip" aria-hidden />
          <div className="eth-sheet__title">{title}</div>
          <IconButton size="lg" tone="ghost" className="eth-mdock__btn eth-mdock__btn--small" label={t("common.close")} icon={<X />} onClick={onClose} />
        </header>
        <div className="eth-sheet__body">{children}</div>
      </section>
    </div>,
    document.body,
  );
}
