/**
 * Copying a folder of the user's computer into the engine's library (`base-136`; web, a
 * remote engine, and desktop drops that arrive as files): read it (`walk.ts`), keep the
 * supported audio (`plan.ts`), check the browser's storage quota (local web engine), then
 * `Browser::ImportFolder` → per file an upload (`stageUpload`) consumed by
 * `Browser::ImportFile` (sub-folders preserved) → `Browser::Rescan`, which indexes it like
 * any library root.
 *
 * Jobs run one after the other, each with a status row (`useFolderImports`): progress in
 * files and bytes, Cancel (stops after the file in flight; what was copied stays, indexed),
 * the skipped count, errors. A file that fails doesn't stop the others.
 */
import { create } from "zustand";
import type { BrowserRoot } from "@/generated";
import { stageUpload } from "@/features/remote/upload";
import { cmd, isCommandFailed, newId, type EngineTransport } from "@/transport";
import { estimateStorage, planImport, quotaProblem, type StorageEstimate } from "./plan";
import type { PickedFolder } from "./walk";

export type FolderJobState = "reading" | "copying" | "done" | "cancelled" | "error";

export interface FolderJob {
  id: string;
  /** The folder's name (`null` while it is being read). */
  name: string | null;
  state: FolderJobState;
  /** Files to copy, and copied so far. */
  total: number;
  done: number;
  bytes: number;
  sent: number;
  skipped: number;
  failed: number;
  /** The new library root, once created. */
  root: string | null;
  error: string | null;
}

interface FolderImportStore {
  jobs: FolderJob[];
  update(id: string, patch: Partial<FolderJob>): void;
  dismiss(id: string): void;
  cancel(id: string): void;
}

const controllers = new Map<string, AbortController>();

export const useFolderImports = create<FolderImportStore>((set) => ({
  jobs: [],
  update: (id, patch) => set((s) => ({ jobs: s.jobs.map((j) => (j.id === id ? { ...j, ...patch } : j)) })),
  dismiss: (id) => set((s) => ({ jobs: s.jobs.filter((j) => j.id !== id) })),
  cancel: (id) => controllers.get(id)?.abort(new DOMException("cancelled", "AbortError")),
}));

export interface FolderImportOptions {
  /** The new root exists (the browser shows it). */
  onRoot?: (root: string) => void;
  /** Test hook: the storage estimate (default: `navigator.storage.estimate()` on the local web engine). */
  estimate?: () => Promise<StorageEstimate | null>;
}

function message(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message;
  return e instanceof Error ? e.message : String(e);
}

const aborted = (e: unknown) => e instanceof DOMException && e.name === "AbortError";

/** Jobs wait for the previous one (one folder copies at a time). */
let queue: Promise<unknown> = Promise.resolve();

/** Import one folder (see the module docs). Resolves with the finished job. */
export function importFolder(
  transport: EngineTransport,
  read: (signal: AbortSignal) => Promise<PickedFolder>,
  opts: FolderImportOptions = {},
): Promise<FolderJob> {
  const id = newId();
  const ctl = new AbortController();
  controllers.set(id, ctl);
  const job: FolderJob = { id, name: null, state: "reading", total: 0, done: 0, bytes: 0, sent: 0, skipped: 0, failed: 0, root: null, error: null };
  useFolderImports.setState((s) => ({ jobs: [...s.jobs, job] }));
  const run = queue.then(() => runJob(transport, id, read, ctl.signal, opts));
  queue = run.catch(() => undefined);
  return run.finally(() => controllers.delete(id));
}

async function runJob(
  transport: EngineTransport,
  id: string,
  read: (signal: AbortSignal) => Promise<PickedFolder>,
  signal: AbortSignal,
  opts: FolderImportOptions,
): Promise<FolderJob> {
  const { update } = useFolderImports.getState();
  const current = () => useFolderImports.getState().jobs.find((j) => j.id === id)!;
  const finish = (patch: Partial<FolderJob>) => {
    update(id, patch);
    return current();
  };
  let root: string | null = null;
  try {
    signal.throwIfAborted();
    const plan = planImport(await read(signal));
    update(id, { name: plan.name, total: plan.files.length, bytes: plan.bytes, skipped: plan.skipped });
    if (plan.files.length === 0) {
      return finish({ state: "error", error: `No supported audio files in “${plan.name}” (WAV, AIFF, FLAC, MP3, OGG).` });
    }
    const estimate = opts.estimate ?? (transport.kind === "wasm" ? estimateStorage : async () => null);
    const problem = quotaProblem(plan, await estimate());
    if (problem) return finish({ state: "error", error: problem });
    signal.throwIfAborted();

    const before = new Set<string>();
    const listed = await transport.send(cmd("Browser", { type: "ListRoots" }));
    if (listed.type === "BrowserRoots") for (const r of listed.roots) before.add(r.id);
    const created = await transport.send(cmd("Browser", { type: "ImportFolder", name: plan.name }));
    const roots: BrowserRoot[] = created.type === "BrowserRoots" ? created.roots : [];
    const added = roots.find((r) => r.kind === "Folder" && !before.has(r.id));
    if (!added) throw new Error("the engine did not create the folder");
    root = added.id;
    update(id, { root, name: added.name, state: "copying" });
    opts.onRoot?.(root);

    let sent = 0;
    for (const f of plan.files) {
      signal.throwIfAborted();
      try {
        await stageUpload(
          transport,
          f.file,
          (n) => update(id, { sent: sent + n }),
          signal,
          (upload) => transport.send(cmd("Browser", { type: "ImportFile", root: root!, path: f.path, upload })),
        );
        update(id, { done: current().done + 1 });
      } catch (e) {
        if (aborted(e) || signal.aborted) throw e;
        console.warn(`[ethereal] ${f.path} was not imported:`, e);
        update(id, { failed: current().failed + 1 });
      }
      sent += f.file.size;
      update(id, { sent });
    }
    await rescan(transport, root);
    const done = current();
    return finish(
      done.failed === done.total ? { state: "error", error: `No file of “${done.name}” could be copied.` } : { state: "done" },
    );
  } catch (e) {
    if (root) await rescan(transport, root);
    if (aborted(e) || signal.aborted) return finish({ state: "cancelled" });
    return finish({ state: "error", error: message(e) });
  }
}

async function rescan(transport: EngineTransport, root: string) {
  await transport.send(cmd("Browser", { type: "Rescan", root })).catch(() => undefined);
}
