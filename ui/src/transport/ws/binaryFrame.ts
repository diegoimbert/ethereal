/**
 * Binary WebSocket frames of the remote-engine protocol, mirroring
 * `crates/ether-protocol/src/remote.rs` (`encode_binary_frame` / `decode_binary_frame`):
 *
 * `[kind: u8][header_len: u32 LE][header: UTF-8 JSON][payload]`
 *
 * - `Bytes` (1): the header message's single base64 byte field (`data`) is sent as `""`; the
 *   payload is its raw bytes.
 * - `Peaks` (2): the header is a `Reply` with `ReplyValue::Peaks` whose `min`/`max` are one
 *   empty array per channel; the payload is, per channel, `n` LE f32 mins then `n` maxes.
 */
import type { BinaryKind } from "@/generated";

export const BINARY_KIND_CODE: Readonly<Record<BinaryKind, number>> = { Bytes: 1, Peaks: 2 };

export function encodeBinaryFrame(kind: BinaryKind, headerJson: string, payload: Uint8Array): Uint8Array {
  const header = new TextEncoder().encode(headerJson);
  const out = new Uint8Array(5 + header.length + payload.length);
  out[0] = BINARY_KIND_CODE[kind];
  new DataView(out.buffer).setUint32(1, header.length, true);
  out.set(header, 5);
  out.set(payload, 5 + header.length);
  return out;
}

export interface BinaryFrame {
  kind: BinaryKind;
  headerJson: string;
  payload: Uint8Array;
}

/** Split a binary frame; throws on truncated frames or unknown kinds. */
export function decodeBinaryFrame(frame: Uint8Array): BinaryFrame {
  if (frame.length < 5) throw new Error("binary frame too short");
  const kind: BinaryKind | undefined = frame[0] === 1 ? "Bytes" : frame[0] === 2 ? "Peaks" : undefined;
  if (!kind) throw new Error(`unknown binary frame kind ${frame[0]}`);
  const len = new DataView(frame.buffer, frame.byteOffset, frame.byteLength).getUint32(1, true);
  if (frame.length - 5 < len) throw new Error("binary frame too short");
  const headerJson = new TextDecoder("utf-8", { fatal: true }).decode(frame.subarray(5, 5 + len));
  return { kind, headerJson, payload: frame.subarray(5 + len) };
}

/** `ether_protocol::remote::PROTOCOL_VERSION` (pinned by the shared vectors). */
export const PROTOCOL_VERSION = 1;

/**
 * `ether_protocol::remote::FEATURE_DEVICE_PARAM_PATCH`: this UI applies `DeviceParam`
 * patch changes, announced in `ClientHello.features`.
 */
export const FEATURE_DEVICE_PARAM_PATCH = "device_param_patch";

/** Decode a `Peaks` payload: per channel `n` LE f32 mins then `n` maxes. */
export function decodePeaksPayload(payload: Uint8Array, channels: number): { min: number[][]; max: number[][] } {
  if (channels <= 0 || payload.length % (8 * channels) !== 0) throw new Error("bad peaks payload");
  const n = payload.length / (8 * channels);
  const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength);
  const read = (start: number) => Array.from({ length: n }, (_, i) => view.getFloat32((start + i) * 4, true));
  const min: number[][] = [];
  const max: number[][] = [];
  for (let c = 0; c < channels; c++) {
    min.push(read(c * 2 * n));
    max.push(read(c * 2 * n + n));
  }
  return { min, max };
}

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/** Two base64 characters (ASCII) for each 12-bit value, in memory order. */
const B64_PAIRS = (() => {
  const t = new Uint16Array(4096);
  const littleEndian = new Uint8Array(new Uint16Array([1]).buffer)[0] === 1;
  for (let i = 0; i < 4096; i++) {
    const hi = B64.charCodeAt(i >> 6);
    const lo = B64.charCodeAt(i & 63);
    t[i] = littleEndian ? hi | (lo << 8) : (hi << 8) | lo;
  }
  return t;
})();
const ASCII = new TextDecoder("ascii");

/**
 * Standard base64 of raw bytes (the JSON form of `Base64Bytes`). Uploads encode every chunk
 * of a file with it: a 12-bit table into a byte buffer (~25x faster than `btoa` over
 * `String.fromCharCode` chunks: an 11 MB project file took 200 ms of the page's thread).
 */
export function bytesToBase64(bytes: Uint8Array): string {
  const n = bytes.length;
  const full = n - (n % 3);
  const out = new Uint8Array(Math.ceil(n / 3) * 4);
  const pairs = new Uint16Array(out.buffer, 0, (full / 3) * 2);
  let j = 0;
  for (let i = 0; i < full; i += 3) {
    const v = (bytes[i]! << 16) | (bytes[i + 1]! << 8) | bytes[i + 2]!;
    pairs[j++] = B64_PAIRS[v >> 12]!;
    pairs[j++] = B64_PAIRS[v & 4095]!;
  }
  if (n - full > 0) {
    const k = (full / 3) * 4;
    const v = (bytes[full]! << 16) | (n - full === 2 ? bytes[full + 1]! << 8 : 0);
    out[k] = B64.charCodeAt(v >> 18);
    out[k + 1] = B64.charCodeAt((v >> 12) & 63);
    out[k + 2] = n - full === 2 ? B64.charCodeAt((v >> 6) & 63) : 61;
    out[k + 3] = 61;
  }
  return ASCII.decode(out);
}
