/** MockTransport: `Capture::*` (v0.3, capture-midi). Pins the contracts-4 stub. */
import { describe, it } from "vitest";
import { cmd } from "../../cmd";
import { createTrack, expectUnsupported, testId, useMock } from "./testUtils";

describe("MockTransport MIDI capture (capture-midi)", () => {
  const f = useMock();

  it("replies Unsupported until the node lands", async () => {
    const track = await createTrack(f, "Midi");
    await expectUnsupported(f, cmd("Capture", { type: "Capture", track, clip: testId(), seed_notes: testId(), adopt_tempo: true }));
    await expectUnsupported(f, cmd("Capture", { type: "Clear" }));
    await expectUnsupported(f, cmd("Capture", { type: "Status" }));
  });
});
