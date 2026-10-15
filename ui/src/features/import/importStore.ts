/**
 * Imports in progress, for the status list (`ImportStatus`): one row per file, with its
 * progress, a cancel action while it can still be cancelled, and its error until dismissed.
 * Successful imports leave the list.
 */
import { create } from "zustand";

export type ImportPhase = "queued" | "uploading" | "decoding" | "placing";

export interface ImportItem {
  id: string;
  name: string;
  phase: ImportPhase;
  /** 0..1 within the current phase, `null` = unknown. */
  progress: number | null;
  /** Failure message (the row stays until dismissed). */
  error: string | null;
  /** Aborts the import; `null` once it can't be cancelled any more. */
  cancel: (() => void) | null;
}

interface ImportState {
  items: readonly ImportItem[];
  put(item: ImportItem): void;
  update(id: string, patch: Partial<ImportItem>): void;
  remove(id: string): void;
  /** Dismiss every failed row. */
  clearErrors(): void;
}

export const useImportStore = create<ImportState>()((set) => ({
  items: [],
  put: (item) => set((s) => ({ items: [...s.items.filter((i) => i.id !== item.id), item] })),
  update: (id, patch) => set((s) => ({ items: s.items.map((i) => (i.id === id ? { ...i, ...patch } : i)) })),
  remove: (id) => set((s) => ({ items: s.items.filter((i) => i.id !== id) })),
  clearErrors: () => set((s) => ({ items: s.items.filter((i) => i.error === null) })),
}));
