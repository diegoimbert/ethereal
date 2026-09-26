import { describe, expect, it } from "vitest";
import { isValidOrderKey, keyBetween, keyForInsert, keysBetween } from "./orderKey";

describe("keyBetween", () => {
  // Vectors from the `fractional-indexing` package test suite.
  it.each([
    [null, null, "a0"],
    [null, "a0", "Zz"],
    [null, "Zz", "Zy"],
    ["a0", null, "a1"],
    ["a1", null, "a2"],
    ["a0", "a1", "a0V"],
    ["a1", "a2", "a1V"],
    ["a0V", "a1", "a0l"],
    ["Zz", "a0", "ZzV"],
    ["Zz", "a1", "a0"],
    [null, "Y00", "Xzzz"],
    ["bzz", null, "c000"],
    ["a0", "a0V", "a0G"],
    ["a0", "a0G", "a08"],
    ["b125", "b129", "b127"],
    ["a0", "a1V", "a1"],
    ["Zz", "a01", "a0"],
    [null, "a0V", "a0"],
    [null, "b999", "b99"],
    ["zzzzzzzzzzzzzzzzzzzzzzzzzzy", null, "zzzzzzzzzzzzzzzzzzzzzzzzzzz"],
    ["zzzzzzzzzzzzzzzzzzzzzzzzzzz", null, "zzzzzzzzzzzzzzzzzzzzzzzzzzzV"],
  ])("keyBetween(%s, %s) = %s", (a, b, expected) => {
    expect(keyBetween(a, b)).toBe(expected);
  });

  it("rejects a >= b and malformed keys", () => {
    expect(() => keyBetween("a1", "a0")).toThrow();
    expect(() => keyBetween("a0", "a0")).toThrow();
    expect(() => keyBetween("a", null)).toThrow();
    expect(() => keyBetween("a0!", null)).toThrow();
    expect(() => keyBetween("a00", null)).toThrow(); // trailing zero
    expect(() => keyBetween("A00000000000000000000000000", null)).toThrow(); // smallest integer
  });

  it("stays ordered under repeated inserts at both ends and in the middle", () => {
    let keys = [keyBetween(null, null)];
    for (let i = 0; i < 200; i++) {
      const r = i % 3;
      if (r === 0) keys = [keyBetween(null, keys[0]!), ...keys];
      else if (r === 1) keys = [...keys, keyBetween(keys.at(-1)!, null)];
      else {
        const j = Math.floor(keys.length / 2);
        keys = [...keys.slice(0, j), keyBetween(keys[j - 1]!, keys[j]!), ...keys.slice(j)];
      }
    }
    const sorted = [...keys].sort((x, y) => (x < y ? -1 : x > y ? 1 : 0));
    expect(keys).toEqual(sorted);
    expect(new Set(keys).size).toBe(keys.length);
    expect(keys.every(isValidOrderKey)).toBe(true);
  });
});

describe("keysBetween", () => {
  it("generates n sorted keys within the bounds", () => {
    for (const [a, b] of [
      [null, null],
      ["a0", null],
      [null, "a0"],
      ["a0", "a1"],
    ] as const) {
      const ks = keysBetween(a, b, 7);
      expect(ks).toHaveLength(7);
      for (let i = 1; i < ks.length; i++) expect(ks[i - 1]! < ks[i]!).toBe(true);
      if (a) expect(ks[0]! > a).toBe(true);
      if (b) expect(ks.at(-1)! < b).toBe(true);
    }
    expect(keysBetween(null, null, 3)).toEqual(["a0", "a1", "a2"]);
  });
});

describe("keyForInsert", () => {
  const sibs = [
    { id: "x", order: "a0" },
    { id: "y", order: "a1" },
  ];
  it("inserts before a sibling or at the end", () => {
    expect(keyForInsert(sibs, "x")).toBe("Zz");
    expect(keyForInsert(sibs, "y")).toBe("a0V");
    expect(keyForInsert(sibs, null)).toBe("a2");
    expect(keyForInsert(sibs, "unknown")).toBe("a2");
    expect(keyForInsert([], null)).toBe("a0");
  });
});
