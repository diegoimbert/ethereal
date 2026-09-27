import { describe, expect, it } from "vitest";
import { decodeBinaryFrame, encodeBinaryFrame } from "./binaryFrame";

describe("binary frames (same vector as ether_protocol::remote tests)", () => {
  it("round-trips and matches the Rust layout", () => {
    const f = encodeBinaryFrame("Bytes", '{"id":1}', new Uint8Array([9, 8, 7]));
    expect([...f.subarray(0, 5)]).toEqual([1, 8, 0, 0, 0]);
    const d = decodeBinaryFrame(f);
    expect(d.kind).toBe("Bytes");
    expect(d.headerJson).toBe('{"id":1}');
    expect([...d.payload]).toEqual([9, 8, 7]);
    expect(() => decodeBinaryFrame(f.subarray(0, 7))).toThrow(/too short/);
    expect(() => decodeBinaryFrame(new Uint8Array([3, 0, 0, 0, 0]))).toThrow(/unknown/);
  });
});
