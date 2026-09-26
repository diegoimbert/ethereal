// Test helpers for the ui-shell features: render under a TransportProvider backed by a
// manual-timer MockTransport, and reset global stores between tests.
import { render, waitFor, type RenderResult } from "@testing-library/react";
import type { ReactElement } from "react";
import { playheadStore, useProjectStore } from "@/state";
import { MockTransport, TransportProvider, type MockTransportOptions } from "@/transport";

export interface MockRender extends RenderResult {
  mock: MockTransport;
}

/** Render `ui` connected to a fresh MockTransport; resolves once the project is loaded. */
export async function renderWithMock(ui: ReactElement, opts: MockTransportOptions = {}): Promise<MockRender> {
  const mock = new MockTransport({ timers: "manual", seed: 7, ...opts });
  const result = render(<TransportProvider transport={mock}>{ui}</TransportProvider>);
  await waitFor(() => {
    if (!useProjectStore.getState().project) throw new Error("not connected");
  });
  return { ...result, mock };
}

export function resetStores(): void {
  useProjectStore.getState().reset();
  playheadStore.reset();
}
