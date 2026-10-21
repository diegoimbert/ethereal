import { create } from "zustand";

/**
 * Why this host can't record (no inputs: the web build's `ListInputs` → `Unsupported`), or
 * null. The recording controls find out; the top bar's record button reads it (disabled, the
 * reason as its tooltip).
 */
export const useRecordSupport = create<{ unsupported: string | null; setUnsupported(reason: string | null): void }>()((set) => ({
  unsupported: null,
  setUnsupported: (unsupported) => set({ unsupported }),
}));
