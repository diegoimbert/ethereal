import { create } from "zustand";

/** A confirmation asked from the Share popover (shown by `ShareConfirmDialog`). */
export interface ConfirmRequest {
  title: string;
  body: string;
  /** The confirm button names the action. */
  action: string;
  danger?: boolean;
  run(): void;
}

interface ConfirmState {
  request: ConfirmRequest | null;
  ask(r: ConfirmRequest): void;
  close(): void;
}

export const useConfirm = create<ConfirmState>()((set) => ({
  request: null,
  ask: (request) => set({ request }),
  close: () => set({ request: null }),
}));
