/** MockTransport: `TimeEdit` (v0.2, time-edits). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport TimeEdit (time-edits)", () => {
  const f = useMock();

  it("replies Unsupported until time-edits lands", async () => {
    await expect(f.mock.send(cmd("TimeEdit", { type: "Split", tracks: [], at: 1, seed: "01J00000000000000000000000" }))).rejects.toMatchObject({ code: "Unsupported" });
  });
});
