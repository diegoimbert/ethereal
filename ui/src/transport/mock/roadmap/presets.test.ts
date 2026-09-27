/** MockTransport: `Preset` (v0.2, presets). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport Preset (presets)", () => {
  const f = useMock();

  it("replies Unsupported until presets lands", async () => {
    await expect(f.mock.send(cmd("Preset", { type: "List", device: null, text: null }))).rejects.toMatchObject({ code: "Unsupported" });
  });
});
