/** MockTransport: `Expression::SetTrackMpe` (v0.3, mpe). Pins the contracts-4 stub. */
import { describe, it } from "vitest";
import { cmd } from "../../cmd";
import { createTrack, expectUnsupported, useMock } from "./testUtils";

describe("MockTransport MPE (mpe)", () => {
  const f = useMock();

  it("replies Unsupported until the node lands", async () => {
    const track = await createTrack(f, "Midi");
    const mpe = { zone: "Lower" as const, member_channels: 15, note_pitch_range: 48, master_pitch_range: 2 };
    await expectUnsupported(f, cmd("Expression", { type: "SetTrackMpe", track, mpe }));
    await expectUnsupported(f, cmd("Expression", { type: "SetTrackMpe", track, mpe: null }));
  });
});
