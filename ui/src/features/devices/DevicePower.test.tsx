import { act, fireEvent, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device } from "@/generated";
import { ContextMenuHost } from "@/kit";
import { playheadStore } from "@/state";
import { cmd, newId, type MockTransport } from "@/transport";
import { flush, renderWithMock, resetStores, store, stubPointerCapture } from "@/features/devices/testUtils";
import { resetAutomationUi, useAutomationUi } from "@/features/automation/uiStore";
import { DeviceChain } from "./index";
import { resetCollapsed } from "./collapsed";

// The on/off button behaves like any param control: right-click for its "Device On" lane
// and reset, and it follows its automation live.
let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  resetAutomationUi();
  resetCollapsed();
  playheadStore.reset();
});

const synth = (): Device => Object.values(store().project!.devices).find((d) => d.name === "Synth")!;
const frame = (position: number) => ({ transport: { position, seconds: 0, playing: true, bpm: 120 } });

async function setup() {
  mock = await renderWithMock(
    <>
      <DeviceChain />
      <ContextMenuHost />
    </>,
  );
  await screen.findByRole("region", { name: "Synth" });
  // Queried each time: the card may re-mount as patches land.
  const card = () => screen.getByRole("region", { name: "Synth" });
  return { card, power: () => card().querySelector(".eth-device__power")! };
}

describe("device on/off button", () => {
  it("right-click opens the param menu: show its automation lane, reset to on", async () => {
    const { power } = await setup();
    const d = synth();
    expect(power()).toHaveAttribute("data-midi-target", JSON.stringify({ type: "Param", target: { type: "DeviceEnabled", device: d.id } }));
    fireEvent.contextMenu(power());
    // The button's own menu, not the header's device menu.
    expect(screen.queryByRole("menuitem", { name: "delete" })).toBeNull();
    expect(screen.getByRole("menuitem", { name: "reset to default" })).toBeDisabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "show automation lane" }));
    expect(useAutomationUi.getState().open.has(d.track)).toBe(true);
    expect(useAutomationUi.getState().shown[d.track]).toContain(`on:${d.id}`);

    // Off: reset turns it back on.
    fireEvent.click(power());
    await flush();
    expect(synth().enabled).toBe(false);
    fireEvent.contextMenu(screen.getByRole("button", { name: "enable Synth" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "reset to default" }));
    await flush();
    expect(synth().enabled).toBe(true);
  });

  it("follows its automation during playback (button and dimmed card)", async () => {
    const { card, power } = await setup();
    const d = synth();
    const lane = newId();
    await act(async () => {
      await mock!.send(
        cmd("Automation", { type: "CreateLane", id: lane, owner: { type: "Track", track: d.track }, target: { type: "DeviceEnabled", device: d.id } }),
      );
      await mock!.send(
        cmd("Automation", {
          type: "AddPoints",
          lane,
          points: [
            { id: newId(), time: 0, value: 1, curve: { type: "Step" } },
            { id: newId(), time: 2, value: 0, curve: { type: "Step" } },
            { id: newId(), time: 4, value: 1, curve: { type: "Step" } },
          ],
        }),
      );
    });
    await flush();
    act(() => playheadStore.setPlayhead(frame(1)));
    expect(power()).toHaveAttribute("aria-pressed", "true");
    expect(power()).toHaveAttribute("data-automated", "true");
    expect(card()).not.toHaveClass("eth-device--bypassed");
    act(() => playheadStore.setPlayhead(frame(3)));
    expect(power()).toHaveAttribute("aria-pressed", "false");
    expect(card()).toHaveClass("eth-device--bypassed");
    act(() => playheadStore.setPlayhead(frame(5)));
    expect(power()).toHaveAttribute("aria-pressed", "true");
    // A click sets the document's switch (what applies where the lane doesn't drive it);
    // the lane keeps driving the button, like an automated knob.
    fireEvent.click(power());
    await flush();
    expect(synth().enabled).toBe(false);
    expect(power()).toHaveAttribute("aria-pressed", "true");
  });
});
