/** MockTransport: `Keymap::*` (v0.3, keymap). Pins the contracts-4 stub. */
import { describe, it } from "vitest";
import { cmd } from "../../cmd";
import { expectUnsupported, useMock } from "./testUtils";

describe("MockTransport keymap (keymap)", () => {
  const f = useMock();

  it("replies Unsupported until the node lands", async () => {
    await expectUnsupported(f, cmd("Keymap", { type: "Get" }));
    await expectUnsupported(f, cmd("Keymap", { type: "Set", keymap: { preset: "AbletonLike", overrides: [] } }));
    await expectUnsupported(f, cmd("Keymap", { type: "Reset" }));
  });
});
