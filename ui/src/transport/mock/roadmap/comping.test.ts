/** MockTransport: `Take` (v0.2, comping). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport Take (comping)", () => {
  const f = useMock();

  it("replies Unsupported until comping lands", async () => {
    await expect(f.mock.send(cmd("Take", { type: "RemoveLane", id: "01J00000000000000000000000" }))).rejects.toMatchObject({ code: "Unsupported" });
  });
});
