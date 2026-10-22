// The installed WebCLAP modules, kept in IndexedDB (database `ether-webclap`): the file as
// installed (re-read and re-compiled at each start), where it came from, and its scan.
// Only the page reads it: neither the plugins nor their UIs can (see docs/PLUGIN-FORMATS.md
// "WebCLAP" > "Security").

import type { ScanResult } from "./scan";
import type { ModuleInfo } from "./wasm";

export interface InstalledModule {
  /** Content hash of `bytes` (`./bundle.ts` `contentId`). */
  id: string;
  /** The file name (or the URL's last part). */
  name: string;
  /** Where it came from: the URL, or `file:<name>`. */
  source: string;
  installedAt: number;
  /** The file as installed (a module or a bundle). */
  bytes: ArrayBuffer;
  info: ModuleInfo;
  scan: ScanResult;
}

const DB = "ether-webclap";
const STORE = "modules";

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE, { keyPath: "id" });
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error("indexeddb failed"));
  });
}

function run<T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  return open().then(
    (db) =>
      new Promise<T>((resolve, reject) => {
        const tx = db.transaction(STORE, mode);
        const req = f(tx.objectStore(STORE));
        tx.oncomplete = () => {
          db.close();
          resolve(req.result);
        };
        tx.onerror = () => {
          db.close();
          reject(tx.error ?? req.error ?? new Error("indexeddb failed"));
        };
      }),
  );
}

export const store = {
  /** Every installed module, oldest first (later installs provide a plugin id first). */
  async list(): Promise<InstalledModule[]> {
    if (typeof indexedDB === "undefined") return [];
    const all = await run("readonly", (s) => s.getAll() as IDBRequest<InstalledModule[]>);
    return all.sort((a, b) => a.installedAt - b.installedAt);
  },
  put(m: InstalledModule): Promise<IDBValidKey> {
    return run("readwrite", (s) => s.put(m));
  },
  remove(id: string): Promise<undefined> {
    return run("readwrite", (s) => s.delete(id));
  },
};
