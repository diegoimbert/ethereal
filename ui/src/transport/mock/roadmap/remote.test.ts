/** MockTransport: uploads (remote-engine). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport uploads", () => {
  const f = useMock();

  it("are unsupported", async () => {
    await expect(f.mock.send(cmd("Media", { type: "BeginUpload", upload: "u", name: "a.wav", size: 3 }))).rejects.toMatchObject({
      code: "Unsupported",
    });
  });
});
