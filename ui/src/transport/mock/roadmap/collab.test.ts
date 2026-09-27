/** MockTransport: collaboration is reserved (collab). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";

describe("MockTransport collab", () => {
  const f = useMock();

  it("is unsupported", async () => {
    await expect(f.mock.send(cmd("Collab", { type: "Leave" }))).rejects.toMatchObject({ code: "Unsupported" });
  });
});
