/** MockTransport: `Freeze` (v0.2, freeze-bounce). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport Freeze (freeze-bounce)", () => {
  const f = useMock();

  it("replies Unsupported until freeze-bounce lands", async () => {
    await expect(f.mock.send(cmd("Freeze", { type: "Unfreeze", track: "01J00000000000000000000000" }))).rejects.toMatchObject({ code: "Unsupported" });
  });
});
