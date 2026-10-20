// runAction: palette and menu entries run an editor action through the editor's own
// key handler, in the editor focused last.
import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { KeyboardEvent } from "react";
import { firstMatch, matchesAction } from "./store";
import { installScopeTracker, resetScopeTracker, runAction } from "./run";

function Editor({ scope, onAction }: { scope: string; onAction(scope: string, id: string): void }) {
  const onKeyDown = (e: KeyboardEvent) => {
    const id = firstMatch(["edit.copy", "edit.delete", "pianoRoll.quantize"], e);
    if (!id) return;
    e.preventDefault();
    onAction(scope, id);
  };
  return <div tabIndex={0} role="group" aria-label={scope} data-keymap-scope={scope} onKeyDown={onKeyDown} />;
}

let uninstall: () => void;
beforeEach(() => {
  uninstall = installScopeTracker();
});
afterEach(() => {
  uninstall();
  resetScopeTracker();
});

describe("runAction", () => {
  it("runs a handled action in the editor focused last, through its key handler", () => {
    const onAction = vi.fn();
    render(
      <>
        <Editor scope="arrangement" onAction={onAction} />
        <Editor scope="pianoRoll" onAction={onAction} />
      </>,
    );
    // Nothing focused yet: the first mounted editor of the action's scopes.
    expect(runAction("edit.copy")).toBe(true);
    expect(onAction).toHaveBeenLastCalledWith("arrangement", "edit.copy");
    screen.getByLabelText("pianoRoll").focus();
    expect(runAction("edit.delete")).toBe(true);
    expect(onAction).toHaveBeenLastCalledWith("pianoRoll", "edit.delete");
    // An action of one scope only goes there, whatever had focus.
    screen.getByLabelText("arrangement").focus();
    expect(runAction("pianoRoll.quantize")).toBe(true);
    expect(onAction).toHaveBeenLastCalledWith("pianoRoll", "pianoRoll.quantize");
    expect(onAction).toHaveBeenCalledTimes(3);
  });

  it("a tagged event matches its action only, never a chord", () => {
    const e = new KeyboardEvent("keydown", { key: "c", metaKey: true, ctrlKey: true });
    Object.defineProperty(e, "ethAction", { value: "edit.delete" });
    expect(matchesAction("edit.delete", e)).toBe(true);
    expect(matchesAction("edit.copy", e)).toBe(false);
  });

  it("returns false for an unknown action or with no editor to run it in", () => {
    expect(runAction("no.such.action")).toBe(false);
    expect(runAction("pianoRoll.quantize")).toBe(false);
  });
});
