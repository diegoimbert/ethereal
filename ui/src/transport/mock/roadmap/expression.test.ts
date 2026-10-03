/** MockTransport: `Expression::*` (v0.3, midi-expression), same semantics as the Rust controller. */
import { beforeEach, describe, expect, it } from "vitest";
import type { ExpressionPoint } from "@/generated";
import { cmd } from "../../cmd";
import { createTrack, project, testId, undo, useMock, trackNamed } from "./testUtils";

const pt = (time: number, value: number): ExpressionPoint => ({ time, value, curve: { type: "Linear" } });

describe("MockTransport expression (midi-expression)", () => {
  const f = useMock();
  let clip = "";
  let note = "";

  beforeEach(async () => {
    const track = await createTrack(f, "Midi");
    clip = testId();
    note = testId();
    await f.mock.send(cmd("Clip", { type: "CreateMidi", id: clip, track, start: 0, length: 4, name: null }));
    await f.mock.send(
      cmd("Note", { type: "Add", clip, notes: [{ id: note, pitch: 60, velocity: 0.8, start: 0, duration: 1 }] }),
    );
  });

  const lanes = () => Object.values(project(f).expression_lanes);
  const exprs = () => Object.values(project(f).note_expressions);

  it("CreateLane: idempotent, one lane per (clip, kind), MIDI clips only, CC 0..=119", async () => {
    const id = testId();
    const create = cmd("Expression", { type: "CreateLane", id, clip, kind: { type: "PitchBend" } });
    await f.mock.send(create);
    await f.mock.send(create);
    // Same kind with another id: no change (the first lane keeps its id).
    await f.mock.send(cmd("Expression", { type: "CreateLane", id: testId(), clip, kind: { type: "PitchBend" } }));
    expect(lanes().map((l) => l.id)).toEqual([id]);
    await f.mock.send(cmd("Expression", { type: "CreateLane", id: testId(), clip, kind: { type: "Cc", controller: 1 } }));
    expect(lanes()).toHaveLength(2);
    await expect(
      f.mock.send(cmd("Expression", { type: "CreateLane", id: testId(), clip, kind: { type: "Cc", controller: 120 } })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    const audio = Object.values(project(f).clips).find((c) => c.track === trackNamed(f, "Drums").id)!;
    await expect(
      f.mock.send(cmd("Expression", { type: "CreateLane", id: testId(), clip: audio.id, kind: { type: "PitchBend" } })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    await undo(f);
    expect(lanes()).toHaveLength(1);
  });

  it("SetPoints validates; ReplaceRange splices; RemoveLane; one undo step each", async () => {
    const lane = testId();
    await f.mock.send(cmd("Expression", { type: "CreateLane", id: lane, clip, kind: { type: "PitchBend" } }));
    await f.mock.send(cmd("Expression", { type: "SetPoints", lane, points: [pt(0, 0), pt(1, 1), pt(2, -1), pt(3, 0)] }));
    for (const bad of [[pt(1, 0), pt(0, 0)], [pt(0, 1.5)], [pt(-1, 0)], [pt(Number.NaN, 0)]]) {
      await expect(f.mock.send(cmd("Expression", { type: "SetPoints", lane, points: bad }))).rejects.toMatchObject({
        code: "InvalidArgument",
      });
    }
    await expect(
      f.mock.send(cmd("Expression", { type: "SetPoints", lane, points: [{ time: 0, value: 0, curve: { type: "Curve", tension: 2 } }] })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    const tooMany = Array.from({ length: 16_385 }, (_, i) => pt(i / 100, 0));
    await expect(f.mock.send(cmd("Expression", { type: "SetPoints", lane, points: tooMany }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });

    await f.mock.send(cmd("Expression", { type: "ReplaceRange", lane, start: 1, end: 3, points: [pt(1.5, 0.5)] }));
    expect(project(f).expression_lanes[lane]!.points).toEqual([pt(0, 0), pt(1.5, 0.5), pt(3, 0)]);
    await expect(
      f.mock.send(cmd("Expression", { type: "ReplaceRange", lane, start: 1, end: 2, points: [pt(2, 0)] })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(
      f.mock.send(cmd("Expression", { type: "ReplaceRange", lane, start: 2, end: 1, points: [] })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });

    await undo(f);
    expect(project(f).expression_lanes[lane]!.points).toHaveLength(4);
    await f.mock.send(cmd("Expression", { type: "RemoveLane", id: lane }));
    expect(lanes()).toHaveLength(0);
    await expect(f.mock.send(cmd("Expression", { type: "RemoveLane", id: lane }))).rejects.toMatchObject({ code: "NotFound" });
    await undo(f);
    expect(project(f).expression_lanes[lane]!.points).toHaveLength(4);
  });

  it("a stroke sent with one gesture is one undo step", async () => {
    const lane = testId();
    await f.mock.send(cmd("Expression", { type: "CreateLane", id: lane, clip, kind: { type: "Cc", controller: 1 } }));
    const gesture = 4242;
    await f.mock.send(cmd("Expression", { type: "ReplaceRange", lane, start: 0, end: 1, points: [pt(0, 0.1)] }), { gesture });
    await f.mock.send(cmd("Expression", { type: "ReplaceRange", lane, start: 0, end: 2, points: [pt(0, 0.1), pt(1, 0.5)] }), {
      gesture,
    });
    await f.mock.send(cmd("Edit", { type: "EndGesture", gesture }));
    expect(project(f).expression_lanes[lane]!.points).toHaveLength(2);
    await undo(f);
    expect(project(f).expression_lanes[lane]!.points).toEqual([]);
  });

  it("SetNoteExpression creates, replaces, removes on empty; ClearNoteExpressions", async () => {
    const id = testId();
    await f.mock.send(cmd("Expression", { type: "SetNoteExpression", id, note, kind: "Pressure", points: [pt(0, 0.2)] }));
    expect(exprs()).toEqual([{ id, note, kind: "Pressure", points: [pt(0, 0.2)] }]);
    // Same kind: replaces the points (the new id is unused).
    await f.mock.send(cmd("Expression", { type: "SetNoteExpression", id: testId(), note, kind: "Pressure", points: [pt(0, 0.9)] }));
    expect(exprs()).toEqual([{ id, note, kind: "Pressure", points: [pt(0, 0.9)] }]);
    await expect(
      f.mock.send(cmd("Expression", { type: "SetNoteExpression", id: testId(), note, kind: "Pressure", points: [pt(0, 2)] })),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    // Pitch takes semitones.
    await f.mock.send(cmd("Expression", { type: "SetNoteExpression", id: testId(), note, kind: "Pitch", points: [pt(0, -12)] }));
    expect(exprs()).toHaveLength(2);
    await f.mock.send(cmd("Expression", { type: "SetNoteExpression", id: testId(), note, kind: "Pressure", points: [] }));
    expect(exprs().map((e) => e.kind)).toEqual(["Pitch"]);
    await f.mock.send(cmd("Expression", { type: "ClearNoteExpressions", notes: [note, testId()], kind: null }));
    expect(exprs()).toEqual([]);
    await undo(f);
    expect(exprs()).toHaveLength(1);
    await expect(
      f.mock.send(cmd("Expression", { type: "SetNoteExpression", id: testId(), note: testId(), kind: "Pressure", points: [pt(0, 0)] })),
    ).rejects.toMatchObject({ code: "NotFound" });
  });

  it("cascades and copies like the Rust document (delete, duplicate)", async () => {
    await f.mock.send(cmd("Expression", { type: "CreateLane", id: testId(), clip, kind: { type: "PitchBend" } }));
    const lane = lanes()[0]!.id;
    await f.mock.send(cmd("Expression", { type: "SetPoints", lane, points: [pt(0, 0.5)] }));
    await f.mock.send(cmd("Expression", { type: "SetNoteExpression", id: testId(), note, kind: "Pressure", points: [pt(0, 0.5)] }));
    // Note duplicate copies its expressions.
    const copy = testId();
    await f.mock.send(cmd("Note", { type: "Duplicate", copies: [{ from: note, new_id: copy }], offset: 1, transpose: 0 }));
    expect(exprs().filter((e) => e.note === copy)).toHaveLength(1);
    // Clip duplicate copies lanes and note expressions.
    const dup = testId();
    await f.mock.send(cmd("Clip", { type: "Duplicate", id: clip, new_id: dup, start: null }));
    expect(lanes().filter((l) => l.clip === dup)).toHaveLength(1);
    expect(exprs()).toHaveLength(4);
    // Removing a note removes its expressions; deleting a clip its lanes and notes' curves.
    await f.mock.send(cmd("Note", { type: "Remove", ids: [copy] }));
    expect(exprs()).toHaveLength(3);
    await f.mock.send(cmd("Clip", { type: "Delete", ids: [clip, dup] }));
    expect(lanes()).toEqual([]);
    expect(exprs()).toEqual([]);
  });
});
