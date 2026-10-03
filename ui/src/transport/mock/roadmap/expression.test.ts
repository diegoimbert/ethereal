/** MockTransport: `Expression::*` (v0.3, midi-expression). Pins the contracts-4 stub. */
import { describe, it } from "vitest";
import { cmd } from "../../cmd";
import { expectUnsupported, testId, useMock } from "./testUtils";

describe("MockTransport expression (midi-expression)", () => {
  const f = useMock();

  it("replies Unsupported until the node lands", async () => {
    const point = { time: 0, value: 0.5, curve: { type: "Linear" as const } };
    await expectUnsupported(f, cmd("Expression", { type: "CreateLane", id: testId(), clip: testId(), kind: { type: "PitchBend" } }));
    await expectUnsupported(f, cmd("Expression", { type: "RemoveLane", id: testId() }));
    await expectUnsupported(f, cmd("Expression", { type: "SetPoints", lane: testId(), points: [point] }));
    await expectUnsupported(f, cmd("Expression", { type: "ReplaceRange", lane: testId(), start: 0, end: 4, points: [] }));
    await expectUnsupported(
      f,
      cmd("Expression", { type: "SetNoteExpression", id: testId(), note: testId(), kind: "Pressure", points: [point] }),
    );
    await expectUnsupported(f, cmd("Expression", { type: "ClearNoteExpressions", notes: [], kind: null }));
  });
});
