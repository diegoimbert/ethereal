// The page's one React root (`#root`): the editor, the startup screens, and any screen an
// extension's web route shows (`@/ext` "web.route") render into it.
import { createRoot, type Root } from "react-dom/client";

let root: Root | null = null;

/** The `#root` element. */
export function rootElement(): HTMLElement {
  const el = document.getElementById("root");
  if (!el) throw new Error("#root element missing");
  return el;
}

/** The page's React root, created on first use. */
export function appRoot(): Root {
  root ??= createRoot(rootElement());
  return root;
}
