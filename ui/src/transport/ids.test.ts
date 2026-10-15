import { isValid } from "ulid";
import { describe, expect, it } from "vitest";
import { newId, newProjectId, uuidv7 } from "./ids";

const UUID_V7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

describe("ids", () => {
  it("newId is a ULID", () => {
    expect(isValid(newId())).toBe(true);
  });

  it("newProjectId is a UUIDv7", () => {
    const a = newProjectId();
    const b = newProjectId();
    expect(a).toMatch(UUID_V7);
    expect(a).not.toBe(b);
  });

  it("encodes the timestamp big-endian in the first 48 bits", () => {
    const ms = 0x0190_1234_5678;
    const id = uuidv7(ms, (bytes) => bytes.fill(0xff));
    expect(id).toBe("01901234-5678-7fff-bfff-ffffffffffff");
    expect(uuidv7(0, (bytes) => bytes.fill(0))).toBe("00000000-0000-7000-8000-000000000000");
  });

  it("sorts by creation time", () => {
    const ids = [1000, 2000, 1_700_000_000_000].map((t) => uuidv7(t));
    expect([...ids].sort()).toEqual(ids);
  });
});
