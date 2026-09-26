// Minimal UTF-8 TextDecoder/TextEncoder for the AudioWorkletGlobalScope, which lacks them
// (the wasm-bindgen glue constructs both at import time). Must be imported before the glue.

interface Scope {
  TextDecoder?: unknown;
  TextEncoder?: unknown;
}
const g = globalThis as unknown as Scope;

class Utf8Decoder {
  decode(bytes?: Uint8Array): string {
    if (!bytes) return "";
    let out = "";
    for (let i = 0; i < bytes.length; ) {
      const b = bytes[i++]!;
      let cp: number;
      if (b < 0x80) cp = b;
      else if (b < 0xe0) cp = ((b & 0x1f) << 6) | (bytes[i++]! & 0x3f);
      else if (b < 0xf0) cp = ((b & 0x0f) << 12) | ((bytes[i++]! & 0x3f) << 6) | (bytes[i++]! & 0x3f);
      else cp = ((b & 0x07) << 18) | ((bytes[i++]! & 0x3f) << 12) | ((bytes[i++]! & 0x3f) << 6) | (bytes[i++]! & 0x3f);
      out += String.fromCodePoint(cp);
    }
    return out;
  }
}

class Utf8Encoder {
  encode(s = ""): Uint8Array {
    const out: number[] = [];
    for (const ch of s) {
      const cp = ch.codePointAt(0)!;
      if (cp < 0x80) out.push(cp);
      else if (cp < 0x800) out.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
      else if (cp < 0x10000) out.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
      else out.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 0x3f), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
    }
    return new Uint8Array(out);
  }
  encodeInto(s: string, view: Uint8Array): { read: number; written: number } {
    const bytes = this.encode(s);
    view.set(bytes);
    return { read: s.length, written: bytes.length };
  }
}

if (typeof g.TextDecoder === "undefined") g.TextDecoder = Utf8Decoder;
if (typeof g.TextEncoder === "undefined") g.TextEncoder = Utf8Encoder;

export {};
