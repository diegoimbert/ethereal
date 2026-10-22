// Small WebAssembly binary helpers for the WebCLAP host:
// - `moduleInfo`: what the JS API can't tell about a module (the limits of an imported
//   memory, whether it is shared or 64-bit), read from the binary's import section;
// - `trampolines`: a tiny module that imports JS functions and re-exports them as wasm
//   functions, so they can be put into a plugin's function table (a table only holds wasm
//   functions; CLAP host callbacks are function-table indices).

/** What the host needs to know about a WebCLAP module beyond `WebAssembly.Module`. */
export interface ModuleInfo {
  /** The imported memory (`env.memory` or any), when the module imports one. */
  memory?: { module: string; name: string; initial: number; maximum?: number; shared: boolean; memory64: boolean };
}

class Reader {
  at = 0;
  constructor(private readonly b: Uint8Array) {}
  byte(): number {
    if (this.at >= this.b.length) throw new Error("truncated wasm module");
    return this.b[this.at++]!;
  }
  u32(): number {
    let result = 0;
    let shift = 0;
    for (;;) {
      const byte = this.byte();
      result += (byte & 0x7f) * 2 ** shift;
      if ((byte & 0x80) === 0) return result;
      shift += 7;
      if (shift > 63) throw new Error("bad number in the wasm module");
    }
  }
  name(): string {
    const n = this.u32();
    const bytes = this.b.slice(this.at, this.at + n);
    this.at += n;
    return new TextDecoder().decode(bytes);
  }
  skipLimits(): void {
    const flags = this.byte();
    this.u32();
    if (flags & 1) this.u32();
  }
}

const MAGIC = [0x00, 0x61, 0x73, 0x6d];

export function isWasm(bytes: Uint8Array): boolean {
  return MAGIC.every((b, i) => bytes[i] === b);
}

/** Read the import section of `bytes` (a wasm binary). */
export function moduleInfo(bytes: Uint8Array): ModuleInfo {
  if (!isWasm(bytes)) throw new Error("not a webassembly module");
  const r = new Reader(bytes);
  r.at = 8;
  const info: ModuleInfo = {};
  while (r.at < bytes.length) {
    const id = r.byte();
    const size = r.u32();
    const end = r.at + size;
    if (id === 2) {
      const count = r.u32();
      for (let i = 0; i < count; i++) {
        const module = r.name();
        const name = r.name();
        const kind = r.byte();
        switch (kind) {
          case 0: // function: type index
            r.u32();
            break;
          case 1: // table: reftype + limits
            r.byte();
            r.skipLimits();
            break;
          case 2: {
            const flags = r.byte();
            const initial = r.u32();
            const maximum = flags & 1 ? r.u32() : undefined;
            info.memory = { module, name, initial, maximum, shared: (flags & 2) !== 0, memory64: (flags & 4) !== 0 };
            break;
          }
          case 3: // global: valtype + mutability
            r.byte();
            r.byte();
            break;
          case 4: // tag
            r.byte();
            r.u32();
            break;
          default:
            throw new Error(`unknown import kind ${kind}`);
        }
      }
      return info;
    }
    r.at = end;
  }
  return info;
}

/** Wasm value types of a host function signature. */
export type ValType = "i32" | "i64" | "f64";
export interface Sig {
  params: readonly ValType[];
  result?: ValType;
}

const VALTYPE: Record<ValType, number> = { i32: 0x7f, i64: 0x7e, f64: 0x7c };

function leb(n: number): number[] {
  const out: number[] = [];
  do {
    let byte = n & 0x7f;
    n >>>= 7;
    if (n !== 0) byte |= 0x80;
    out.push(byte);
  } while (n !== 0);
  return out;
}

function str(s: string): number[] {
  const b = [...new TextEncoder().encode(s)];
  return [...leb(b.length), ...b];
}

function section(id: number, body: number[]): number[] {
  return [id, ...leb(body.length), ...body];
}

const cache = new Map<string, WebAssembly.Module>();

/**
 * A module importing `h.f<i>` with `sigs[i]` and exporting each as `f<i>`. Compiled once
 * per signature list.
 */
export function trampolineModule(sigs: readonly Sig[]): WebAssembly.Module {
  const key = sigs.map((s) => `${s.params.join(",")}>${s.result ?? ""}`).join(";");
  const hit = cache.get(key);
  if (hit) return hit;
  const types: number[] = [...leb(sigs.length)];
  for (const s of sigs) {
    types.push(0x60, ...leb(s.params.length), ...s.params.map((p) => VALTYPE[p]));
    types.push(...(s.result ? [1, VALTYPE[s.result]] : [0]));
  }
  const imports: number[] = [...leb(sigs.length)];
  const exports: number[] = [...leb(sigs.length)];
  sigs.forEach((_, i) => {
    imports.push(...str("h"), ...str(`f${i}`), 0x00, ...leb(i));
    exports.push(...str(`f${i}`), 0x00, ...leb(i));
  });
  const bytes = new Uint8Array([...MAGIC, 0x01, 0x00, 0x00, 0x00, ...section(1, types), ...section(2, imports), ...section(7, exports)]);
  const module = new WebAssembly.Module(bytes);
  cache.set(key, module);
  return module;
}

/** Wasm functions calling `fns[i]` (with `sigs[i]`). */
export function trampolines(sigs: readonly Sig[], fns: readonly ((...args: never[]) => unknown)[]): WebAssembly.ExportValue[] {
  const h: Record<string, (...args: never[]) => unknown> = {};
  fns.forEach((f, i) => (h[`f${i}`] = f));
  const instance = new WebAssembly.Instance(trampolineModule(sigs), { h } as unknown as WebAssembly.Imports);
  return sigs.map((_, i) => instance.exports[`f${i}`]!);
}
