/** MockTransport: base behaviour touched by contracts-2. */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { project, trackNamed, useMock } from "./testUtils";

describe("MockTransport shared", () => {
  const f = useMock();

  it("creates new audio clips unwarped (Repitch, like the controller)", async () => {
    const media = Object.values(project(f).media)[0]!;
    const id = newId();
    await f.mock.send(cmd("Clip", { type: "CreateAudio", id, track: trackNamed(f, "Drums").id, start: 32, media: media.id }));
    const c = project(f).clips[id]!;
    expect(c.content.type === "Audio" && c.content.warp).toEqual({ enabled: false, mode: "Repitch", source_bpm: null });
  });
});
