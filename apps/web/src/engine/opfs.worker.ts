// OPFS helper Worker: performs file operations for the controller Worker, whose Rust
// `ProjectStore` is synchronous while OPFS is async. The controller posts an `FsRequest` on
// a MessagePort and blocks on `Atomics.wait`; this worker answers through the shared fs
// buffer in chunks (see ./fsWire.ts).
import { FS_DONE, FS_ERROR, respond } from "./fsWire";
import { FS_HEADER_BYTES, type FsRequest } from "./protocol";

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

/** `FileSystemHandle.move()` (Chromium 110+, Firefox 111+, Safari 16.4+; not in lib.dom). */
type Movable = FileSystemFileHandle & { move?: (dir: FileSystemDirectoryHandle, name: string) => Promise<void> };

/**
 * Replace `to` with `from`. OPFS has no guaranteed atomic replace: `move()` (overwriting
 * where the browser allows it, else after removing the destination), or copy + remove where
 * `move()` is missing. Either way `to` can be missing or truncated for a moment, but `from`
 * (the store's complete temp file) survives until the end, and `WebStore::load` falls
 * back to it, so an interrupted save never loses the document.
 */
async function rename(from: string, to: string): Promise<void> {
  const src = (await fileHandle(from)) as Movable;
  const [toDir, toName] = await locate(to, true);
  if (typeof src.move === "function") {
    try {
      await src.move(toDir, toName);
      return;
    } catch (e) {
      if (isLocked(e)) throw e;
      // Some implementations refuse to overwrite: remove the destination and retry.
      await toDir.removeEntry(toName).catch(() => undefined);
      await src.move(toDir, toName);
      return;
    }
  }
  await writeFile(to, await readFile(from));
  const [dir, name] = await locate(from, false);
  await dir.removeEntry(name);
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
    case "rename":
      if (!req.to) throw new FsError("InvalidPath", "rename without destination");
      await rename(req.path, req.to);
      return EMPTY;
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

/** Another tab holds the file (sync access handles are exclusive). */
function isLocked(e: unknown): boolean {
  const name = (e as { name?: string } | null)?.name;
  return name === "NoModificationAllowedError" || name === "InvalidStateError";
}

function errorPayload(e: unknown): Uint8Array {
  if (e instanceof FsError) return json({ code: e.code, message: e.message });
  if (isLocked(e)) {
    return json({
      code: "Io",
      message: "the project file is locked: Ethereal is probably open in another tab of this browser",
    });
  }
  const message = e instanceof Error ? e.message : String(e);
  return json({ code: isNotFound(e) ? "NotFound" : "Io", message });
}

/** A caller that stops acknowledging has timed out; don't block forever on it. */
const ACK_TIMEOUT_MS = 30_000;

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
        respond(ctrl, data, req.seq, await perform(req), FS_DONE, ACK_TIMEOUT_MS);
      } catch (err) {
        respond(ctrl, data, req.seq, errorPayload(err), FS_ERROR, ACK_TIMEOUT_MS);
      }
    });
  };
};
