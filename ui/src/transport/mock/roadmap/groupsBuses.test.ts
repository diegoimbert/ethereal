/** MockTransport: `Track` (v0.2, groups-buses). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport Track (groups-buses)", () => {
  const f = useMock();

  it("replies Unsupported until groups-buses lands", async () => {
    await expect(f.mock.send(cmd("Track", { type: "GroupSelected", ids: [], group: "01J00000000000000000000000", name: null }))).rejects.toMatchObject({ code: "Unsupported" });
  });
});
