// Test helpers shared by the mixer and devices tests (not imported by app code).
import { act, fireEvent, render, waitFor } from "@testing-library/react";
import type { Project, Track } from "@/generated";
import { useProjectStore, useSelectionStore } from "@/state";
import { MockTransport, TransportProvider } from "@/transport";
import type { ReactNode } from "react";
import { expect } from "vitest";

export const store = () => useProjectStore.getState();

export function trackByName(name: string): Track {
  const t = Object.values(store().project!.tracks).find((x) => x.name === name);
  if (!t) throw new Error(`no track ${name}`);
  return t;
}

/** Render `ui` against a fresh MockTransport (manual timers) and wait for the project. */
export async function renderWithMock(ui: ReactNode, project?: Project): Promise<MockTransport> {
  const mock = new MockTransport({ timers: "manual", seed: 1, ...(project ? { project } : {}) });
  render(<TransportProvider transport={mock}>{ui}</TransportProvider>);
  await waitFor(() => expect(store().project).not.toBeNull());
  return mock;
}

export function resetStores(mock: MockTransport | undefined): void {
  mock?.dispose();
  store().reset();
  useSelectionStore.getState().selectTrack(null);
}

/** jsdom lacks pointer capture; the kit's drag hook calls it. */
export function stubPointerCapture(): void {
  const proto = HTMLElement.prototype as unknown as Record<string, unknown>;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
  proto.hasPointerCapture ??= () => false;
}

/** Drag a kit control upward by `steps` × `dy` pixels (one pointer drag = one gesture). */
export async function dragUp(el: Element, steps = 3, dy = 10): Promise<void> {
  await act(async () => {
    fireEvent.pointerDown(el, { button: 0, pointerId: 1, clientY: 200 });
  });
  for (let i = 1; i <= steps; i++) {
    await act(async () => {
      fireEvent.pointerMove(el, { pointerId: 1, clientY: 200 - i * dy });
    });
  }
  await act(async () => {
    fireEvent.pointerUp(el, { pointerId: 1, clientY: 200 - steps * dy });
  });
}

/** Let pending sends (and their patches) settle. */
export async function flush(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  });
}
