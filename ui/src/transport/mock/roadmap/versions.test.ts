/** MockTransport: `Version::*` (v0.3, project-versions). Pins the contracts-4 stub. */
import { describe, it } from "vitest";
import { cmd } from "../../cmd";
import { expectUnsupported, testId, useMock } from "./testUtils";

describe("MockTransport project versions (project-versions)", () => {
  const f = useMock();

  it("replies Unsupported until the node lands", async () => {
    await expectUnsupported(f, cmd("Version", { type: "List" }));
    await expectUnsupported(f, cmd("Version", { type: "Create", name: "Before mixdown" }));
    await expectUnsupported(f, cmd("Version", { type: "Restore", version: "v1" }));
    await expectUnsupported(f, cmd("Version", { type: "Delete", version: "v1" }));
    await expectUnsupported(f, cmd("Version", { type: "Rename", version: "v1", name: null }));
    await expectUnsupported(f, cmd("Version", { type: "Compare", version: "v1", against: null }));
    await expectUnsupported(f, cmd("Version", { type: "ListRecoverable" }));
    await expectUnsupported(f, cmd("Version", { type: "Recover", project: testId() }));
    await expectUnsupported(f, cmd("Version", { type: "DiscardRecovery", project: testId() }));
  });
});
