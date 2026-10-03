/** MockTransport: `History::*` (v0.3, undo-history). Pins the contracts-4 stub. */
import { describe, it } from "vitest";
import { cmd } from "../../cmd";
import { expectUnsupported, useMock } from "./testUtils";

describe("MockTransport undo history (undo-history)", () => {
  const f = useMock();

  it("replies Unsupported until the node lands", async () => {
    await expectUnsupported(f, cmd("History", { type: "List" }));
    await expectUnsupported(f, cmd("History", { type: "JumpTo", step: null }));
    await expectUnsupported(f, cmd("History", { type: "SetCheckpoint", step: 1, name: "Verse done" }));
  });
});
