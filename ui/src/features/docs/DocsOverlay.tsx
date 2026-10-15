// The docs over the app (desktop): a full-window layer with its own (memory) router.
import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { DocsApp } from "./DocsApp";
import { useMemoryRouter } from "./router";
import type { DocsLocation } from "./route";

export default function DocsOverlay({ initial, onClose }: { initial: DocsLocation; onClose(): void }) {
  const router = useMemoryRouter(initial);
  const layer = useRef<HTMLDivElement>(null);
  // Focus moves into the docs (keys go to them) and back to where it was when they close.
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    layer.current?.querySelector<HTMLElement>(".eth-docs__scroll")?.focus();
    return () => prev?.focus?.();
  }, []);
  return createPortal(
    <div ref={layer} className="eth-docs-layer">
      <DocsApp router={router} onClose={onClose} />
    </div>,
    document.body,
  );
}
