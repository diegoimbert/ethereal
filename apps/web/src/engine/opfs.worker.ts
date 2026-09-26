// OPFS helper Worker: performs file operations for the controller Worker, whose Rust
// `ProjectStore` is synchronous while OPFS is async. The controller posts an `FsRequest` on
// a MessagePort and blocks on `Atomics.wait`; this worker answers through the shared fs
// buffer in chunks (see ./syncFs.ts and ./protocol.ts).
import {
  FS_CHUNK,
  FS_CHUNK_BYTES,
  FS_DONE,
  FS_ERROR,
  FS_HEADER_BYTES,
  type FsRequest,
} from "./protocol";

interface Scope {
  onmessage: ((e: MessageEvent<{ type: "init"; port: MessagePort; buffer: SharedArrayBuffer }>) => void) | null;
}
const scope = self as unknown as Scope;

/** Everything lives under one OPFS folder (the origin is already per dev instance). */
const ROOT = "ethereal";
const encoder = new TextEncoder();

class FsError extends Error {
  constructor(
    readonly code: "NotFound" | "InvalidPath" | "Io",
    message: string,
  ) {
    super(message);
  }
}

function parts(path: string): string[] {
  const p = path.split("/").filter((c) => c.length > 0);
  if (p.some((c) => c === "." || c === "..")) throw new FsError("InvalidPath", path);
  return p;
}

async function root(): Promise<FileSystemDirectoryHandle> {
  const top = await navigator.storage.getDirectory();
  return top.getDirectoryHandle(ROOT, { create: true });
}

async function dirOf(names: string[], create: boolean): Promise<FileSystemDirectoryHandle> {
  let dir = await root();
  for (const n of names) dir = await dir.getDirectoryHandle(n, { create });
  return dir;
}

/** Parent directory handle + leaf name. */
async function locate(path: string, create: boolean): Promise<[FileSystemDirectoryHandle, string]> {
  const p = parts(path);
  const name = p.pop();
  if (name === undefined) throw new FsError("InvalidPath", "empty path");
  return [await dirOf(p, create), name];
}

async function fileHandle(path: string, create = false): Promise<FileSystemFileHandle> {
  const [dir, name] = await locate(path, create);
  return dir.getFileHandle(name, { create });
}

interface Entry {
  name: string;
  is_dir: boolean;
  size: number;
  modified_ms: number;
}

async function entryOf(name: string, handle: FileSystemHandle): Promise<Entry> {
  if (handle.kind === "directory") return { name, is_dir: true, size: 0, modified_ms: 0 };
  const file = await (handle as FileSystemFileHandle).getFile();
  return { name, is_dir: false, size: file.size, modified_ms: file.lastModified };
}

/** `FileSystemSyncAccessHandle` (worker-only; not in lib.dom). */
interface SyncAccessHandle {
  truncate(size: number): void;
  write(buffer: Uint8Array, opts: { at: number }): number;
  flush(): void;
  close(): void;
}

async function writeFile(path: string, bytes: Uint8Array): Promise<void> {
  const fh = (await fileHandle(path, true)) as FileSystemFileHandle & {
    createSyncAccessHandle(): Promise<SyncAccessHandle>;
  };
  // Sync access handles are available in dedicated workers on every OPFS browser.
  const access = await fh.createSyncAccessHandle();
  try {
    access.truncate(0);
    access.write(bytes, { at: 0 });
    access.flush();
  } finally {
    access.close();
  }
}

async function readFile(path: string): Promise<Uint8Array> {
  const file = await (await fileHandle(path)).getFile();
  return new Uint8Array(await file.arrayBuffer());
}

const EMPTY = new Uint8Array(0);
const json = (v: unknown) => encoder.encode(JSON.stringify(v));

async function perform(req: FsRequest): Promise<Uint8Array> {
  switch (req.op) {
    case "read":
      return readFile(req.path);
    case "write":
      await writeFile(req.path, req.bytes ?? EMPTY);
      return EMPTY;
    case "rename": {
      if (!req.to) throw new FsError("InvalidPath", "rename without destination");
      // Not atomic on OPFS in general; write-then-remove keeps the destination valid at
      // every step (the store writes a temp file first).
      await writeFile(req.to, await readFile(req.path));
      const [dir, name] = await locate(req.path, false);
      await dir.removeEntry(name);
      return EMPTY;
    }
    case "list": {
      const dir = await dirOf(parts(req.path), false);
      const out: Entry[] = [];
      for await (const [name, handle] of dir.entries()) out.push(await entryOf(name, handle));
      return json(out);
    }
    case "mkdir":
      await dirOf(parts(req.path), true);
      return EMPTY;
    case "remove": {
      const [dir, name] = await locate(req.path, false);
      await dir.removeEntry(name, { recursive: true });
      return EMPTY;
    }
    case "stat": {
      const p = parts(req.path);
      if (p.length === 0) return json({ name: "", is_dir: true, size: 0, modified_ms: 0 });
      try {
        const [dir, name] = await locate(req.path, false);
        for await (const [n, handle] of dir.entries()) if (n === name) return json(await entryOf(n, handle));
        return json(null);
      } catch (e) {
        if (isNotFound(e)) return json(null);
        throw e;
      }
    }
  }
}

function isNotFound(e: unknown): boolean {
  const name = (e as { name?: string } | null)?.name;
  return name === "NotFoundError" || name === "TypeMismatchError";
}

function errorPayload(e: unknown): Uint8Array {
  if (e instanceof FsError) return json({ code: e.code, message: e.message });
  const message = e instanceof Error ? e.message : String(e);
  return json({ code: isNotFound(e) ? "NotFound" : "Io", message });
}

function respond(ctrl: Int32Array, data: Uint8Array, bytes: Uint8Array, final: number): void {
  let off = 0;
  for (;;) {
    const n = Math.min(FS_CHUNK_BYTES, bytes.length - off);
    data.set(bytes.subarray(off, off + n));
    Atomics.store(ctrl, 1, n);
    off += n;
    const last = off >= bytes.length;
    Atomics.store(ctrl, 0, last ? final : FS_CHUNK);
    Atomics.notify(ctrl, 0);
    if (last) return;
    // Wait for the controller to copy the chunk out (it resets the state to IDLE).
    Atomics.wait(ctrl, 0, FS_CHUNK);
  }
}

scope.onmessage = (e) => {
  if (e.data.type !== "init") return;
  const { port, buffer } = e.data;
  const ctrl = new Int32Array(buffer, 0, FS_HEADER_BYTES / 4);
  const data = new Uint8Array(buffer, FS_HEADER_BYTES);
  let queue = Promise.resolve();
  port.onmessage = (m: MessageEvent<FsRequest>) => {
    const req = m.data;
    queue = queue.then(async () => {
      try {
        respond(ctrl, data, await perform(req), FS_DONE);
      } catch (err) {
        respond(ctrl, data, errorPayload(err), FS_ERROR);
      }
    });
  };
};
