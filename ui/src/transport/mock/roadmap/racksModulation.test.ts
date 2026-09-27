/** MockTransport: `Rack` / `Modulation` (v0.2, racks-modulation). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport racks and modulation (racks-modulation)", () => {
  const f = useMock();

  it("lists modulator kinds and replies Unsupported to edits until racks-modulation lands", async () => {
    const kinds = await f.mock.send(cmd("Modulation", { type: "ListModulatorKinds" }));
    expect(kinds).toMatchObject({ type: "ModulatorKinds" });
    await expect(
      f.mock.send(cmd("Rack", { type: "RemoveChain", id: "01J00000000000000000000000" })),
    ).rejects.toMatchObject({ code: "Unsupported" });
  });
});
