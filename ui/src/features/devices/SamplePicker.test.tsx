import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device } from "@/generated";
import { devicesOfTrack } from "@/state";
import { cmd, type MockTransport } from "@/transport";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { pickDevice } from "@/features/devices/testing";
import { DeviceChain } from "./index";
import { resetCollapsed } from "./collapsed";
import { DeviceWindowsRoot, resetDeviceWindows, useDeviceWindows } from "./window";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  resetDeviceWindows();
  resetCollapsed();
});

const sampler = (): Device => devicesOfTrack(store().project!, trackByName("Keys").id).find((d) => d.name === "sampler")!;
const sampleOf = (d: Device) => (d.kind.type === "Builtin" && d.kind.device.type === "Sampler" ? d.kind.device.sample : undefined);

/** Keys gets a Sampler (its window closed), returns the chain. */
async function renderWithSampler() {
  mock = await renderWithMock(
    <>
      <DeviceChain />
      <DeviceWindowsRoot />
    </>,
  );
  await screen.findByRole("region", { name: "Synth" });
  await pickDevice(screen.getByRole("button", { name: "add device" }), "sampler");
  await flush();
  act(() => useDeviceWindows.getState().setOpen(sampler().id, false));
  await flush();
}

const openWindow = () => {
  act(() => useDeviceWindows.getState().setOpen(sampler().id, true));
  return screen.getByRole("dialog", { name: "sampler window" });
};
const picker = () => screen.queryByRole("dialog", { name: "select a sample" });

describe("sample picker", () => {
  it("opens by itself when a Sampler window without a sample opens", async () => {
    await renderWithSampler();
    expect(picker()).toBeNull();
    const win = openWindow();
    await waitFor(() => expect(within(win).queryByText("loading…")).toBeNull());
    expect(within(win).getByTestId("sample-slot").textContent).toMatch(/drop or select a sample/i);
    await waitFor(() => expect(picker()).not.toBeNull());
    expect(within(picker()!).getByRole("searchbox", { name: "search samples" })).toHaveFocus();
  });

  it("a click on the slot opens it; typing filters and Enter loads the first sample (one undo step)", async () => {
    await renderWithSampler();
    const win = openWindow();
    await waitFor(() => expect(picker()).not.toBeNull());
    // Escape closes it; a click on the slot opens it again.
    fireEvent.keyDown(within(picker()!).getByRole("searchbox"), { key: "Escape" });
    await waitFor(() => expect(picker()).toBeNull());
    fireEvent.click(within(win).getByRole("button", { name: /drop or select a sample/i }));
    const panel = await waitFor(() => {
      const p = picker();
      expect(p).not.toBeNull();
      return p!;
    });
    const search = within(panel).getByRole("searchbox", { name: "search samples" });
    fireEvent.change(search, { target: { value: "kick" } });
    await waitFor(() => expect(within(panel).getAllByRole("button", { name: /kick/i }).length).toBeGreaterThan(0));
    const rows = within(panel).getByRole("list", { name: "samples" }).querySelectorAll("[data-key]");
    for (const r of rows) expect(r.getAttribute("aria-label")).toMatch(/kick/i);
    const first = rows[0]!.getAttribute("aria-label")!;
    const undoBefore = store().history.undo_label;
    fireEvent.keyDown(search, { key: "Enter" });
    await flush();
    await waitFor(() => expect(sampleOf(sampler())).toBeTruthy());
    expect(store().project!.media[sampleOf(sampler())!]!.name).toMatch(new RegExp(first.replace(/\.[^.]+$/, "").replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
    expect(within(win).getByTestId("sample-slot").textContent).toMatch(/kick/i);
    await waitFor(() => expect(picker()).toBeNull());
    // Import + SetSample: one undo step.
    expect(store().history.undo_label).not.toBe(undoBefore);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(sampleOf(sampler())).toBeNull();
  });

  it("does not open by itself when the Sampler has a sample", async () => {
    await renderWithSampler();
    const win = openWindow();
    await waitFor(() => expect(picker()).not.toBeNull());
    const panel = picker()!;
    await waitFor(() => expect(within(panel).getByRole("list", { name: "samples" }).querySelector("[data-key]")).not.toBeNull());
    fireEvent.click(within(panel).getByRole("list", { name: "samples" }).querySelector("[data-key]")!);
    await waitFor(() => expect(sampleOf(sampler())).toBeTruthy());
    act(() => useDeviceWindows.getState().setOpen(sampler().id, false));
    await flush();
    expect(win.isConnected).toBe(false);
    openWindow();
    await new Promise((r) => setTimeout(r, 300));
    expect(picker()).toBeNull();
  });
});
