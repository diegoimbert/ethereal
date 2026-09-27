/** MockTransport: `Browser` (v0.2, browser-v2). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport Browser (browser-v2)", () => {
  const f = useMock();

  it("replies Unsupported until browser-v2 lands", async () => {
    await expect(f.mock.send(cmd("Browser", { type: "ListRoots" }))).rejects.toMatchObject({ code: "Unsupported" });
  });
});
