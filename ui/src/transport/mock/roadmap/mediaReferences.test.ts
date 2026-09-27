/** MockTransport: `MediaRef` (v0.2, media-references). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport MediaRef (media-references)", () => {
  const f = useMock();

  it("replies Unsupported until media-references lands", async () => {
    await expect(f.mock.send(cmd("MediaRef", { type: "ListMissing" }))).rejects.toMatchObject({ code: "Unsupported" });
  });
});
