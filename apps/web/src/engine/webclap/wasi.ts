// A minimal `wasi_snapshot_preview1` for WebCLAP modules: what wasi-libc needs to start
// and to log. No file system (no preopened directories: opening files fails), no
// environment, no arguments, no sockets. Standard output and error go to the host's log.
// Anything else answers `ENOSYS`.
//
// The plugin reaches nothing else through it: it is the only system interface a plugin
// gets, besides the CLAP host callbacks.

const ESUCCESS = 0;
const EBADF = 8;
const EINVAL = 28;
const ENOSYS = 52;
const ESPIPE = 70;

/** Thrown by `proc_exit`: the instance is unusable afterwards. */
export class WasiExit extends Error {
  constructor(readonly code: number) {
    super(`the plugin exited (code ${code})`);
  }
}

export interface WasiOptions {
  memory: () => WebAssembly.Memory;
  /** A line written to stdout (fd 1) or stderr (fd 2). */
  log: (fd: number, line: string) => void;
}

function random(buf: Uint8Array): void {
  const c = (globalThis as { crypto?: Crypto }).crypto;
  if (c?.getRandomValues) {
    // `getRandomValues` refuses shared memory and more than 64 KiB at a time.
    for (let at = 0; at < buf.length; at += 65536) {
      const part = new Uint8Array(Math.min(65536, buf.length - at));
      c.getRandomValues(part);
      buf.set(part, at);
    }
    return;
  }
  for (let i = 0; i < buf.length; i++) buf[i] = Math.floor(Math.random() * 256);
}

/** The `wasi_snapshot_preview1` import object for one instance. */
export function wasiImports(opts: WasiOptions, wanted: readonly string[]): Record<string, (...args: never[]) => unknown> {
  const view = () => new DataView(opts.memory().buffer);
  const bytes = () => new Uint8Array(opts.memory().buffer);
  const pending: Record<number, string> = { 1: "", 2: "" };
  const decoder = new TextDecoder();

  const impl: Record<string, (...args: never[]) => unknown> = {
    args_sizes_get: (argc: number, size: number) => {
      view().setUint32(argc, 0, true);
      view().setUint32(size, 0, true);
      return ESUCCESS;
    },
    args_get: () => ESUCCESS,
    environ_sizes_get: (count: number, size: number) => {
      view().setUint32(count, 0, true);
      view().setUint32(size, 0, true);
      return ESUCCESS;
    },
    environ_get: () => ESUCCESS,
    clock_res_get: (_id: number, out: number) => {
      view().setUint32(out, 1000, true);
      view().setUint32(out + 4, 0, true);
      return ESUCCESS;
    },
    clock_time_get: (id: number, _precision: bigint, out: number) => {
      const ms = id === 0 ? Date.now() : (globalThis.performance?.now() ?? Date.now());
      const ns = ms * 1e6;
      view().setUint32(out, ns % 2 ** 32, true);
      view().setUint32(out + 4, Math.floor(ns / 2 ** 32), true);
      return ESUCCESS;
    },
    fd_write: (fd: number, iovs: number, count: number, written: number) => {
      if (fd !== 1 && fd !== 2) return EBADF;
      const v = view();
      let total = 0;
      let text = "";
      for (let i = 0; i < count; i++) {
        const ptr = v.getUint32(iovs + 8 * i, true);
        const len = v.getUint32(iovs + 8 * i + 4, true);
        text += decoder.decode(bytes().slice(ptr, ptr + len));
        total += len;
      }
      const lines = (pending[fd] + text).split("\n");
      pending[fd] = lines.pop() ?? "";
      for (const line of lines) opts.log(fd, line);
      if (pending[fd]!.length > 4096) {
        opts.log(fd, pending[fd]!);
        pending[fd] = "";
      }
      view().setUint32(written, total, true);
      return ESUCCESS;
    },
    fd_read: (fd: number, _iovs: number, _count: number, read: number) => {
      if (fd !== 0) return EBADF;
      view().setUint32(read, 0, true);
      return ESUCCESS;
    },
    fd_close: (fd: number) => (fd <= 2 ? ESUCCESS : EBADF),
    fd_seek: (fd: number) => (fd <= 2 ? ESPIPE : EBADF),
    fd_fdstat_get: (fd: number, out: number) => {
      if (fd > 2) return EBADF;
      const v = view();
      for (let i = 0; i < 24; i++) v.setUint8(out + i, 0);
      v.setUint8(out, 2); // character device
      return ESUCCESS;
    },
    fd_fdstat_set_flags: (fd: number) => (fd <= 2 ? ESUCCESS : EBADF),
    fd_prestat_get: () => EBADF,
    fd_prestat_dir_name: () => EBADF,
    path_open: () => ENOSYS,
    random_get: (ptr: number, len: number) => {
      if (len < 0) return EINVAL;
      const tmp = new Uint8Array(len);
      random(tmp);
      bytes().set(tmp, ptr);
      return ESUCCESS;
    },
    sched_yield: () => ESUCCESS,
    proc_exit: (code: number) => {
      throw new WasiExit(code);
    },
  };
  const out: Record<string, (...args: never[]) => unknown> = {};
  for (const name of wanted) out[name] = impl[name] ?? (() => ENOSYS);
  return out;
}
