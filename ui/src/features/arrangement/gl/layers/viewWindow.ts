import type { ViewWindow } from "../scene";

/**
 * Per-key build windows of view-dependent layer data (polylines culled to the view, curves
 * sampled at the zoom): `token(key, win)` returns the window last built for `key` while the
 * view stays inside it at the same zoom, else `win` (a new token: the layer rebuilds for it).
 * Use the token in a layer's `deps` and build for it.
 *
 * The renderer's window is the view plus a view width of margin on each side at the settled
 * zoom (`ViewWindow`), so panning doesn't rebuild until it leaves the margin.
 */
export class WindowTokens {
  private built = new Map<string, ViewWindow>();

  token(key: string, win: ViewWindow): ViewWindow {
    const b = this.built.get(key);
    const span = (win.to - win.from) / 3;
    const from = win.from + span;
    const to = win.to - span;
    if (b && b.ppb === win.ppb && b.dpr === win.dpr && from >= b.from - 1e-9 && to <= b.to + 1e-9) return b;
    this.built.set(key, win);
    return win;
  }
}
