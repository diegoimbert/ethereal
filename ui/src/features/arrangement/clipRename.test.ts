import { describe, expect, it } from "vitest";
import type { Project } from "@/generated";
import { renameClipInPlace, renameClipsCommand, useClipRename } from "./clipRename";

// Just what renaming reads: the clips' names and content, and an audio clip's media name.
const project = {
  clips: {
    m: { id: "m", name: "Bass", content: { type: "Midi" } },
    a: { id: "a", name: "Take", content: { type: "Audio", media: "s" } },
    b: { id: "b", name: "Verse", content: { type: "Midi" } },
  },
  media: { s: { name: "kick.wav" } },
} as unknown as Project;

describe("renameClipsCommand", () => {
  it("renames one clip with a plain Clip::Rename", () => {
    expect(renameClipsCommand(project, ["m"], " Lead ")).toEqual({ domain: "Clip", command: { type: "Rename", id: "m", name: "Lead" } });
  });

  it("renames several as one keyed undo step, leaving out the ones already named so", () => {
    const edit = renameClipsCommand(project, ["m", "a", "b", "gone"], "Verse");
    expect(edit).toEqual({
      domain: "Edit",
      command: {
        type: "Batch",
        label: "rename clips",
        key: { id: "undo.clips.rename", args: { count: 2 } },
        commands: [
          { domain: "Clip", command: { type: "Rename", id: "m", name: "Verse" } },
          { domain: "Clip", command: { type: "Rename", id: "a", name: "Verse" } },
        ],
      },
    });
  });

  it("an empty name gives each clip its own default name; nothing to change gives null", () => {
    const edit = renameClipsCommand(project, ["m", "a"], "  ");
    expect(edit?.domain === "Edit" && edit.command.type === "Batch" && edit.command.commands).toEqual([
      { domain: "Clip", command: { type: "Rename", id: "m", name: "" } },
      { domain: "Clip", command: { type: "Rename", id: "a", name: "kick" } },
    ]);
    expect(renameClipsCommand(project, ["b"], "Verse")).toBeNull();
  });
});

describe("renameClipInPlace", () => {
  it("keeps the clicked clip among the renamed ones, and clears on stop", () => {
    renameClipInPlace("m", ["a", "b"]);
    expect(useClipRename.getState()).toMatchObject({ clip: "m", ids: ["m", "a", "b"] });
    renameClipInPlace("a");
    expect(useClipRename.getState().ids).toEqual(["a"]);
    renameClipInPlace(null);
    expect(useClipRename.getState()).toMatchObject({ clip: null, ids: [] });
  });
});
