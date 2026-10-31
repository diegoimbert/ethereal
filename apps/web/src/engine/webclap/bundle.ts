// WebCLAP files: a self-contained module (`.wclap.wasm` / `.wasm`) or a bundle
// (`.wclap.tar.gz`): a directory with `module.wasm` and its resources (a web UI's files).
// Format: https://github.com/WebCLAP (the "WCLAP" description).

import { isWasm } from "./wasm";

export interface Bundle {
  wasm: Uint8Array;
  /** The bundle's other files, by path relative to the bundle root (`html/index.html`). */
  files: Map<string, Uint8Array>;
}

/** Largest bundle accepted (unpacked). */
export const MAX_BUNDLE_BYTES = 256 * 1024 * 1024;

/** `res`'s body, refused ("too big") past `max` bytes (announced or received). */
export async function readCapped(res: Response, max: number): Promise<Uint8Array> {
  const tooBig = (): Error => new Error("the file is too big");
  const announced = Number(res.headers.get("content-length") ?? NaN);
  if (announced > max) throw tooBig();
  const reader = res.body?.getReader();
  const parts: Uint8Array[] = [];
  let total = 0;
  while (reader) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.length;
    if (total > max) {
      await reader.cancel();
      throw tooBig();
    }
    parts.push(value);
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

async function gunzip(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new DecompressionStream("gzip"));
  const reader = stream.getReader();
  const parts: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.length;
    if (total > MAX_BUNDLE_BYTES) {
      await reader.cancel();
      throw new Error("the bundle is too big");
    }
    parts.push(value);
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

function text(b: Uint8Array, start: number, len: number): string {
  let end = start;
  while (end < start + len && b[end] !== 0) end++;
  return new TextDecoder().decode(b.subarray(start, end));
}

function octal(b: Uint8Array, start: number, len: number): number {
  const s = text(b, start, len).trim();
  return s ? parseInt(s, 8) : 0;
}

/** The regular files of a (ustar, pax or GNU) tar archive. */
export function untar(b: Uint8Array): Map<string, Uint8Array> {
  const files = new Map<string, Uint8Array>();
  let at = 0;
  let longName: string | null = null;
  let paxPath: string | null = null;
  while (at + 512 <= b.length) {
    if (b.subarray(at, at + 512).every((x) => x === 0)) break;
    const name = text(b, at, 100);
    const size = octal(b, at + 124, 12);
    const type = String.fromCharCode(b[at + 156] || 0x30);
    const magic = text(b, at + 257, 6);
    const prefix = magic.startsWith("ustar") ? text(b, at + 345, 155) : "";
    const body = b.subarray(at + 512, at + 512 + size);
    at += 512 + Math.ceil(size / 512) * 512;
    if (type === "L") {
      longName = text(body, 0, body.length);
      continue;
    }
    if (type === "x") {
      for (const line of new TextDecoder().decode(body).split("\n")) {
        const m = /^\d+ path=(.*)$/.exec(line);
        if (m) paxPath = m[1]!;
      }
      continue;
    }
    if (type === "g") continue;
    const path = paxPath ?? longName ?? (prefix ? `${prefix}/${name}` : name);
    paxPath = null;
    longName = null;
    if (type !== "0" && type !== "\0" && type !== "7") continue;
    files.set(path.replace(/^\.\//, ""), body.slice());
  }
  return files;
}

/** Strip the bundle's own directory (`plugin.wclap/…`) and macOS metadata files. */
function relativeFiles(all: Map<string, Uint8Array>): Map<string, Uint8Array> {
  const keep = [...all].filter(([p]) => !p.split("/").some((part) => part.startsWith("._") || part === "__MACOSX"));
  const wasm = keep.map(([p]) => p).filter((p) => p === "module.wasm" || p.endsWith("/module.wasm"));
  wasm.sort((a, b) => a.length - b.length);
  const root = wasm[0] ? wasm[0].slice(0, -"module.wasm".length) : "";
  const out = new Map<string, Uint8Array>();
  for (const [p, bytes] of keep) if (p.startsWith(root)) out.set(p.slice(root.length), bytes);
  return out;
}

/** Read a downloaded or picked WebCLAP file. */
export async function readBundle(bytes: Uint8Array): Promise<Bundle> {
  if (isWasm(bytes)) return { wasm: bytes, files: new Map() };
  if (bytes[0] === 0x1f && bytes[1] === 0x8b) {
    const files = relativeFiles(untar(await gunzip(bytes)));
    const wasm = files.get("module.wasm");
    if (!wasm || !isWasm(wasm)) throw new Error("the bundle has no module.wasm");
    files.delete("module.wasm");
    return { wasm, files };
  }
  throw new Error("not a webclap file (a .wasm module or a .tar.gz bundle)");
}

/** The module's id: the first 16 bytes of the SHA-256 of the file, hex. */
export async function contentId(bytes: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes as BufferSource));
  return [...digest.subarray(0, 16)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
