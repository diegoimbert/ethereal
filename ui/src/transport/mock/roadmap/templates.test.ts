/** MockTransport: `Template::*` (v0.3, templates). Pins the contracts-4 stub. */
import { describe, it } from "vitest";
import type { PresetMeta } from "@/generated";
import { cmd } from "../../cmd";
import { createTrack, expectUnsupported, testId, useMock } from "./testUtils";

describe("MockTransport templates (templates)", () => {
  const f = useMock();

  it("replies Unsupported until the node lands", async () => {
    const track = await createTrack(f, "Midi");
    const meta: PresetMeta = { tags: [], author: null, description: null };
    await expectUnsupported(f, cmd("Template", { type: "List", kind: null }));
    await expectUnsupported(f, cmd("Template", { type: "SaveProject", name: "Default", meta, overwrite: false }));
    await expectUnsupported(f, cmd("Template", { type: "SaveTracks", tracks: [track], name: "Keys", meta, overwrite: false }));
    await expectUnsupported(f, cmd("Template", { type: "Insert", template: "keys", seed: testId(), parent: null, before: null }));
    await expectUnsupported(f, cmd("Template", { type: "NewProject", id: testId(), name: "Song", template: null }));
    await expectUnsupported(f, cmd("Template", { type: "Rename", template: "keys", name: "Keys 2" }));
    await expectUnsupported(f, cmd("Template", { type: "Delete", template: "keys" }));
    await expectUnsupported(f, cmd("Template", { type: "SetDefault", template: null }));
  });
});
