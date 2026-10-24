/** The editor drawer follows a multi-clip selection (so its edits apply to every selected clip). */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Clip, Project } from "@/generated";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { bindEditedClipToSelection } from "./actions";

const clip = (id: string, type: "Audio" | "Midi"): Clip => ({ id, content: { type } }) as unknown as Clip;

let off: () => void;
beforeEach(() => {
  const clips = Object.fromEntries([clip("a1", "Audio"), clip("a2", "Audio"), clip("a3", "Audio"), clip("m1", "Midi")].map((c) => [c.id, c]));
  useProjectStore.setState({ project: { clips } as unknown as Project });
  itemSelection.getState().clear();
  useEditorStore.setState({ clip: null });
  off = bindEditedClipToSelection();
});
afterEach(() => {
  off();
  itemSelection.getState().clear();
  useEditorStore.setState({ clip: null });
  useProjectStore.getState().reset();
});

const select = (ids: string[], mode: "replace" | "add" = "replace") => itemSelection.getState().select("clip", ids, mode);
const edited = () => useEditorStore.getState().clip;

describe("bindEditedClipToSelection", () => {
  it("a selection of several clips without the shown one: the drawer shows one of them, of its kind", () => {
    useEditorStore.getState().openClip("a1");
    select(["m1", "a2", "a3"]);
    expect(edited()).toBe("a3");
    // Now inside the selection: adding more keeps it.
    select(["m1"], "add");
    expect(edited()).toBe("a3");
  });

  it("no clip of the shown kind: the last added one", () => {
    useEditorStore.getState().openClip("m1");
    select(["a1", "a2"]);
    expect(edited()).toBe("a2");
  });

  it("leaves single selections, a closed editor and a shown clip already selected alone", () => {
    select(["a2", "a3"]);
    expect(edited()).toBeNull();
    useEditorStore.getState().openClip("a1");
    select(["a2"]);
    expect(edited()).toBe("a1");
    select(["a1", "a3"]);
    expect(edited()).toBe("a1");
  });
});
