import { describe, expect, it } from "vitest";
import type { Note } from "@/generated";
import { isNoteTable, noteTable, withNotes } from "./noteTable";

const note = (i: number, clip = "C1"): Note => ({
  id: `01HZZZ${i.toString(36).padStart(20, "0")}`.toUpperCase(),
  clip,
  pitch: 40 + (i % 40),
  velocity: 0.5,
  release_velocity: 0.5,
  start: i * 0.25,
  duration: 0.25,
  muted: false,
});

/** Every read a plain object supports gives the same answer on the table. */
function expectSame(table: Record<string, Note>, plain: Record<string, Note>) {
  expect(Object.keys(table).sort()).toEqual(Object.keys(plain).sort());
  expect(Object.values(table).length).toBe(Object.values(plain).length);
  for (const [k, v] of Object.entries(plain)) {
    expect(table[k]).toBe(v);
    expect(k in table).toBe(true);
    expect(Object.hasOwn(table, k)).toBe(true);
  }
  expect(table["01HNOPE"]).toBeUndefined();
  expect("01HNOPE" in table).toBe(false);
  expect({ ...table }).toEqual(plain);
  expect(JSON.parse(JSON.stringify(table))).toEqual(JSON.parse(JSON.stringify(plain)));
  expect(table).toEqual(plain);
}

describe("noteTable", () => {
  it("reads like the plain table through random upserts, moves and removes", () => {
    const plain: Record<string, Note> = {};
    for (let i = 0; i < 3000; i++) plain[note(i).id] = note(i);
    let table = noteTable(plain);
    expect(isNoteTable(table)).toBe(true);
    expectSame(table, plain);
    let x = 7;
    const rnd = (n: number) => {
      x = (x * 1103515245 + 12345) & 0x7fffffff;
      return x % n;
    };
    for (let step = 0; step < 50; step++) {
      const ops: (readonly [string, Note | undefined])[] = [];
      for (let k = 0; k < 1 + rnd(40); k++) {
        const i = rnd(3500);
        const r = rnd(3);
        if (r === 0) {
          ops.push([note(i).id, undefined]);
          delete plain[note(i).id];
        } else {
          const n = { ...note(i, r === 1 ? "C1" : "C2"), pitch: rnd(127) };
          ops.push([n.id, n]);
          plain[n.id] = n;
        }
      }
      const before = table;
      const snapshot = { ...before };
      table = withNotes(table, ops);
      expectSame(table, plain);
      // The old table is unchanged (persistent).
      expect({ ...before }).toEqual(snapshot);
    }
  });

  it("refuses writes like a frozen object", () => {
    const t = noteTable({ [note(1).id]: note(1) });
    expect(() => {
      "use strict";
      (t as Record<string, Note>)[note(2).id] = note(2);
    }).toThrow(TypeError);
    expect(() => {
      delete (t as Record<string, Note>)[note(1).id];
    }).toThrow(TypeError);
    expect(t[note(1).id]).toEqual(note(1));
  });
});
