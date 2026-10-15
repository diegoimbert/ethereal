import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Project, Track } from "@/generated";
import { Sheet } from "./Sheet";
import { siblingMove } from "./trackMoves";

afterEach(cleanup);

describe("Sheet", () => {
  it("a dialog with a close button; the scrim and Escape close it", () => {
    const onClose = vi.fn();
    render(
      <Sheet open onClose={onClose} label="Track" title="Bass">
        <p>body</p>
      </Sheet>,
    );
    expect(screen.getByRole("dialog", { name: "Track" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "close" }));
    fireEvent.click(document.querySelector(".eth-sheet__scrim")!);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(3);
  });

  it("the system back closes it (a history entry, same URL); closing otherwise takes the entry back", async () => {
    const onClose = vi.fn();
    const url = location.href;
    const length = history.length;
    const { rerender } = render(
      <Sheet open onClose={onClose} label="Track" title="Bass">
        <p>body</p>
      </Sheet>,
    );
    expect(history.length).toBe(length + 1);
    expect(location.href).toBe(url);
    // Back: the entry before the sheet's.
    act(() => {
      history.replaceState({}, "", url);
      window.dispatchEvent(new PopStateEvent("popstate", { state: {} }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    // Closed by the app while its entry is on top: back is called to remove it.
    const back = vi.spyOn(history, "back").mockImplementation(() => undefined);
    rerender(
      <Sheet open={false} onClose={onClose} label="Track" title="Bass">
        <p>body</p>
      </Sheet>,
    );
    rerender(
      <Sheet open onClose={onClose} label="Track" title="Bass">
        <p>body</p>
      </Sheet>,
    );
    rerender(
      <Sheet open={false} onClose={onClose} label="Track" title="Bass">
        <p>body</p>
      </Sheet>,
    );
    expect(back).toHaveBeenCalledTimes(1);
    back.mockRestore();
  });
});

describe("siblingMove", () => {
  const track = (id: string, order: string, parent: string | null = null, kind: Track["kind"] = "Audio") => ({ id, kind, order, parent }) as unknown as Track;
  const project = (tracks: Track[]) => ({ tracks: Object.fromEntries(tracks.map((t) => [t.id, t])) }) as unknown as Project;
  const p = project([
    track("a", "a0"),
    track("g", "a1", null, "Group"),
    track("g1", "b0", "g"),
    track("g2", "b1", "g"),
    track("c", "a2"),
    track("m", "z", null, "Master"),
  ]);

  it("moves among its siblings: up before the previous one, down before the one after the next", () => {
    expect(siblingMove(p, "c", -1)).toEqual({ domain: "Track", command: { type: "Move", id: "c", parent: null, before: "g" } });
    expect(siblingMove(p, "a", 1)).toEqual({ domain: "Track", command: { type: "Move", id: "a", parent: null, before: "c" } });
    expect(siblingMove(p, "g", 1)).toEqual({ domain: "Track", command: { type: "Move", id: "g", parent: null, before: null } });
    expect(siblingMove(p, "g2", -1)).toEqual({ domain: "Track", command: { type: "Move", id: "g2", parent: "g", before: "g1" } });
  });

  it("nothing past the ends, nor for the master", () => {
    expect(siblingMove(p, "a", -1)).toBeNull();
    expect(siblingMove(p, "c", 1)).toBeNull();
    expect(siblingMove(p, "g1", -1)).toBeNull();
    expect(siblingMove(p, "m", -1)).toBeNull();
  });

  it("returns move among the returns only (#791)", () => {
    const q = project([
      track("a", "a0"),
      track("b", "a1"),
      track("r1", "c0", null, "Return"),
      track("r2", "c1", null, "Return"),
      track("m", "z", null, "Master"),
    ]);
    expect(siblingMove(q, "r2", -1)).toEqual({ domain: "Track", command: { type: "Move", id: "r2", parent: null, before: "r1" } });
    expect(siblingMove(q, "r1", 1)).toEqual({ domain: "Track", command: { type: "Move", id: "r1", parent: null, before: null } });
    // The first return doesn't go up among the regular tracks, the last regular one not down among the returns.
    expect(siblingMove(q, "r1", -1)).toBeNull();
    expect(siblingMove(q, "b", 1)).toBeNull();
  });
});
