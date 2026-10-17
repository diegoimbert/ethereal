import { afterEach, describe, expect, it } from "vitest";
import { DEFAULT_NOTE_VELOCITY, newNoteVelocity, resetLastVelocity, touchNote } from "./lastVelocity";

afterEach(resetLastVelocity);

describe("newNoteVelocity", () => {
  it("is the default before any note was touched", () => {
    expect(newNoteVelocity({ a: { velocity: 0.3 } })).toBe(DEFAULT_NOTE_VELOCITY);
    expect(newNoteVelocity(null)).toBe(DEFAULT_NOTE_VELOCITY);
  });

  it("follows the last touched note, including later edits of its velocity", () => {
    touchNote({ id: "a", velocity: 0.3 });
    touchNote({ id: "b", velocity: 0.6 });
    expect(newNoteVelocity({ a: { velocity: 0.3 }, b: { velocity: 0.6 } })).toBe(0.6);
    expect(newNoteVelocity({ a: { velocity: 0.3 }, b: { velocity: 0.9 } })).toBe(0.9);
  });

  it("keeps the velocity a note had when touched once it is gone", () => {
    touchNote({ id: "a", velocity: 0.4 });
    expect(newNoteVelocity({})).toBe(0.4);
    expect(newNoteVelocity(undefined)).toBe(0.4);
  });
});
