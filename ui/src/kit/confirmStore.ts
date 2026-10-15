// The pending confirmation request, asked through `useConfirm.getState().ask(...)` and
// rendered by `<ConfirmDialog />` (mounted once by the shell).
import type { ReactNode } from "react";
import { create } from "zustand";

export interface ConfirmRequest {
  title: ReactNode;
  body: ReactNode;
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
