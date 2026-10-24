/** Stretch drags (ctrl/cmd+shift on a clip edge): preview math and the command. */
import { describe, expect, it } from "vitest";
import type { Clip, Command } from "@/generated";
import { isStretchModifier, stretchCommand, stretchPreview, type DragInput } from "./editMath";

const clip = (id: string, start: number, length: number, extra: Partial<Clip> = {}): Clip =>
  ({
    id,
    track: "t",
    start,
    name: id,
    color: null,
    muted: false,
    length,
    offset: 0,
    looping: { enabled: false, start: 0, end: length },
    content: { type: "Midi" },
    ...extra,
  }) as Clip;

const grid = (b: number) => Math.round(b);
const input = (clips: Clip[], anchor = clips[0]!.id): DragInput => ({ clips, anchor, rows: [], snap: grid, sourceLength: () => 1 });

describe("stretchPreview", () => {
  it("end edge: the start stays, the content scales with the length (no media limit)", () => {
    const a = clip("a", 4, 4, { offset: 1, looping: { enabled: true, start: 1, end: 3 } });
    const p = stretchPreview(input([a]), "resize-end", 3.8).get("a")!;
    expect(p).toMatchObject({ start: 4, length: 8, offset: 2, stretch: 2 });
    expect(p.looping).toMatchObject({ enabled: true, start: 2, end: 6 });
  });

  it("start edge: the end stays; the anchor's snapped delta applies to every clip", () => {
    const a = clip("a", 4, 4);
    const b = clip("b", 10, 2);
    const p = stretchPreview(input([a, b]), "resize-start", 2.2);
    expect(p.get("a")).toMatchObject({ start: 6, length: 2, stretch: 0.5 });
    expect(p.get("b")).toMatchObject({ start: 12 - 1 / 16, length: 1 / 16 });
    // Never before 0.
    expect(stretchPreview(input([a]), "resize-start", -10).get("a")).toMatchObject({ start: 0, length: 8, stretch: 2 });
  });

  it("commits one Stretch per changed clip, one undo step", () => {
    const a = clip("a", 4, 4);
    const b = clip("b", 10, 2);
    const one = stretchCommand([a], stretchPreview(input([a]), "resize-end", 2));
    expect(one).toMatchObject({ domain: "Clip", command: { type: "Stretch", id: "a", start: 4, length: 6 } });
    const both = stretchCommand([a, b], stretchPreview(input([a, b]), "resize-end", 2)) as Command & { command: { type: string; commands: Command[] } };
    expect(both.command.type).toBe("Batch");
    expect(both.command.commands).toHaveLength(2);
    expect(stretchCommand([a], stretchPreview(input([a]), "resize-end", 0.2))).toBeNull();
  });

  it("modifiers: ctrl+shift or cmd+shift", () => {
    expect(isStretchModifier({ shiftKey: true, ctrlKey: true, metaKey: false })).toBe(true);
    expect(isStretchModifier({ shiftKey: true, ctrlKey: false, metaKey: true })).toBe(true);
    expect(isStretchModifier({ shiftKey: true, ctrlKey: false, metaKey: false })).toBe(false);
    expect(isStretchModifier({ shiftKey: false, ctrlKey: true, metaKey: false })).toBe(false);
  });
});
