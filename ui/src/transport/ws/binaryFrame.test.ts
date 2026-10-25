import { describe, expect, it } from "vitest";
import { bytesToBase64, decodeBinaryFrame, decodePeaksPayload, encodeBinaryFrame, FEATURE_DEVICE_PARAM_PATCH, PROTOCOL_VERSION } from "./binaryFrame";
import vectors from "./binaryFrame.vectors.json";

const hex = (s: string) => new Uint8Array((s.match(/../g) ?? []).map((b) => parseInt(b, 16)));

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

describe("shared Rust ↔ TS vectors (crates/ether-server/tests/frame_vectors.rs)", () => {
  it("pins the protocol version", () => {
    expect(PROTOCOL_VERSION).toBe(vectors.protocol_version);
    expect(FEATURE_DEVICE_PARAM_PATCH).toBe(vectors.feature_device_param_patch);
  });

  for (const v of vectors.vectors) {
    it(v.name, () => {
      const payload = hex(v.payload_hex);
      const frame = encodeBinaryFrame(v.kind as "Bytes" | "Peaks", v.header, payload);
      expect([...frame]).toEqual([...hex(v.frame_hex)]);
      const d = decodeBinaryFrame(hex(v.frame_hex));
      expect(d.kind).toBe(v.kind);
      expect(d.headerJson).toBe(v.header);
      expect([...d.payload]).toEqual([...payload]);
      if ("peaks" in v && v.peaks) {
        expect(decodePeaksPayload(d.payload, v.peaks.min.length)).toEqual(v.peaks);
      }
    });
  }

  it("rejects invalid frames", () => {
    for (const v of vectors.invalid) expect(() => decodeBinaryFrame(hex(v.frame_hex)), v.name).toThrow();
  });

  it("base64 matches the JSON form", () => {
    expect(bytesToBase64(new Uint8Array([1, 2, 3]))).toBe("AQID");
    expect(bytesToBase64(new Uint8Array())).toBe("");
  });
});

describe("bytesToBase64", () => {
  it("matches btoa for every length and byte value", () => {
    const reference = (b: Uint8Array) => btoa(Array.from(b, (x) => String.fromCharCode(x)).join(""));
    const all = new Uint8Array(1031);
    for (let i = 0; i < all.length; i++) all[i] = (i * 2654435761) >>> 24;
    for (let i = 0; i < 256; i++) all[i] = i;
    for (let len = 0; len <= 64; len++) expect(bytesToBase64(all.subarray(0, len))).toBe(reference(all.subarray(0, len)));
    expect(bytesToBase64(all)).toBe(reference(all));
    expect(bytesToBase64(all.subarray(5, 700))).toBe(reference(all.subarray(5, 700)));
  });
});
