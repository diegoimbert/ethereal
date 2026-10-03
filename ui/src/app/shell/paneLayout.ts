import type { PaneState } from "./shellStore";

type Pane = Pick<PaneState, "open" | "pinned" | "size">;

/**
 * Where the floating panes go, as the CSS variables the workspace sets (px, read by
 * App.css). Unpinned panes float inset from the workspace edges by `gap`; pinned ones sit
 * flush with the workspace edges (lined up with the arrangement) and keep `gap` between
 * them and the arrangement, whose padding (`*-reserved`) makes room for them.
 *
 * The drawer spans between the open side panes (panes never overlap each other): unpinned,
 * it floats `gap` away from them and from the workspace edges; pinned, it lines up with the
 * arrangement's edges where a side pane is pinned and with the workspace edge where none is
 * open.
 */
export function paneLayout(left: Pane, right: Pane, bottom: Pane, gap: number): Record<string, string> {
  const reserved = (p: Pane) => (p.open && p.pinned ? p.size + gap : 0);
  // Room an open side pane takes from the drawer's edge, gap included.
  const beside = (p: Pane) => (p.open ? p.size + (p.pinned ? gap : 2 * gap) : 0);
  // Unpinned drawer: inset by a gap from whatever it is next to. Pinned: flush.
  const drawerEdge = (side: Pane) =>
    bottom.pinned ? beside(side) : (side.open ? side.size + gap : 0) + gap;
  const v = (n: number) => `${n}px`;
  return {
    "--pane-left-size": v(left.size),
    "--pane-right-size": v(right.size),
    "--pane-bottom-size": v(bottom.size),
    "--pane-left-reserved": v(reserved(left)),
    "--pane-right-reserved": v(reserved(right)),
    "--pane-bottom-reserved": v(reserved(bottom)),
    // Drawer's left/right offsets from the workspace edges (the left one after the rail).
    "--pane-bottom-left": v(drawerEdge(left)),
    "--pane-bottom-right": v(drawerEdge(right)),
  };
}
