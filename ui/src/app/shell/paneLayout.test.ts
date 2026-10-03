import { describe, expect, it } from "vitest";
import { paneLayout } from "./paneLayout";

const GAP = 12;
const pane = (open: boolean, pinned: boolean, size: number) => ({ open, pinned, size });
const closed = pane(false, false, 300);

describe("paneLayout", () => {
  it("a pinned pane reserves its size plus one gap (flush with the workspace edge)", () => {
    const v = paneLayout(pane(true, true, 280), pane(true, true, 320), pane(true, true, 200), GAP);
    expect(v["--pane-left-reserved"]).toBe("292px");
    expect(v["--pane-right-reserved"]).toBe("332px");
    expect(v["--pane-bottom-reserved"]).toBe("212px");
  });

  it("unpinned or closed panes reserve nothing", () => {
    const v = paneLayout(pane(true, false, 280), closed, pane(false, true, 200), GAP);
    expect(v["--pane-left-reserved"]).toBe("0px");
    expect(v["--pane-right-reserved"]).toBe("0px");
    expect(v["--pane-bottom-reserved"]).toBe("0px");
  });

  it("an unpinned drawer floats a gap away from the side panes and the workspace edges", () => {
    const floating = pane(true, false, 200);
    expect(paneLayout(closed, closed, floating, GAP)["--pane-bottom-left"]).toBe(`${GAP}px`);
    expect(paneLayout(pane(true, false, 280), closed, floating, GAP)["--pane-bottom-left"]).toBe(`${280 + 2 * GAP}px`);
    // A pinned left pane: the drawer stays inset a gap from the arrangement's left edge.
    expect(paneLayout(pane(true, true, 280), closed, floating, GAP)["--pane-bottom-left"]).toBe(`${280 + 2 * GAP}px`);
    expect(paneLayout(closed, pane(true, true, 320), floating, GAP)["--pane-bottom-right"]).toBe(`${320 + 2 * GAP}px`);
  });

  it("a pinned drawer lines up with the arrangement's edges", () => {
    const pinned = pane(true, true, 200);
    // No side pane: flush with the workspace edges.
    let v = paneLayout(closed, closed, pinned, GAP);
    expect(v["--pane-bottom-left"]).toBe("0px");
    expect(v["--pane-bottom-right"]).toBe("0px");
    // Pinned side panes: the arrangement's edges (= their reserved space).
    v = paneLayout(pane(true, true, 280), pane(true, true, 320), pinned, GAP);
    expect(v["--pane-bottom-left"]).toBe(v["--pane-left-reserved"]);
    expect(v["--pane-bottom-right"]).toBe(v["--pane-right-reserved"]);
    // A floating side pane: the drawer keeps clear of it, a gap away.
    v = paneLayout(pane(true, false, 280), closed, pinned, GAP);
    expect(v["--pane-bottom-left"]).toBe(`${280 + 2 * GAP}px`);
  });
});
