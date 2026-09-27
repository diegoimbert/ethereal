/** MockTransport: chat and pinned notes (collab-social contract stub, base-62). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { project, useMock } from "./testUtils";

describe("MockTransport social (stub)", () => {
  const f = useMock();

  it("chat and pinned notes reply Unsupported until collab-social lands", async () => {
    await expect(f.mock.send(cmd("Chat", { type: "Send", id: newId(), text: "hi" }))).rejects.toMatchObject({ code: "Unsupported" });
    const add = cmd("PinnedNote", {
      type: "Add",
      id: newId(),
      position: { beats: 4, track: null, y: 0 },
      text: "look here",
      author_name: null,
    });
    await expect(f.mock.send(add)).rejects.toMatchObject({ code: "Unsupported" });
    expect(project(f).chat).toEqual({});
    expect(project(f).pinned_notes).toEqual({});
  });
});
