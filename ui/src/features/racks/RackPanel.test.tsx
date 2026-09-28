/** Racks and modulation in the device chain (MockTransport): chains, chain devices, a
 * modulator mapped onto a chain device's knob (depth ring + chip), macros as sources. */
import { cleanup, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { DeviceChain } from "@/features/devices";
import { flush, renderWithMock, resetStores, stubPointerCapture, store } from "@/features/mixer/testUtils";
import { useContextMenuStore } from "@/kit";
import { pickOption } from "@/kit/testing";
import { cmd, type MockTransport } from "@/transport";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  cleanup();
  resetStores(mock);
  mock = undefined;
});

const region = (name: string) => screen.getByRole("region", { name });

async function rackWithSynth() {
  mock = await renderWithMock(<DeviceChain />);
  await screen.findAllByRole("slider", { name: "Cutoff" });
  pickOption(screen.getByRole("combobox", { name: "Add device" }), { value: "InstrumentRack" });
  await flush();
  const rack = await screen.findByRole("region", { name: "Instrument Rack" });
  fireEvent.click(within(rack).getByRole("button", { name: "Chain" }));
  await flush();
  const chains = within(rack).getByRole("listbox", { name: "Chains" });
  expect(within(chains).getByRole("option", { name: "Chain 1" })).toHaveAttribute("aria-selected", "true");
  pickOption(within(rack).getByRole("combobox", { name: "Add device to Chain 1" }), { value: "Delay" });
  await flush();
  return rack;
}

describe("RackPanel", () => {
  it("adds chains and chain devices, which stay off the track chain", async () => {
    const rack = await rackWithSynth();
    const delay = await within(rack).findByRole("region", { name: "Delay" });
    expect(delay).toBeInTheDocument();
    const p = store().project!;
    const d = Object.values(p.devices).find((x) => x.name === "Delay" && x.chain != null);
    expect(d).toBeDefined();
    // The track chain list shows the rack only once, not its chain devices.
    const list = screen.getByRole("list", { name: "Keys devices" });
    expect(within(list).getAllByRole("region").filter((r) => r.parentElement?.parentElement === list)).toHaveLength(3);
    // Mute and solo.
    fireEvent.click(within(rack).getByRole("button", { name: "Mute Chain 1" }));
    await flush();
    expect(Object.values(store().project!.rack_chains)[0]!.mute).toBe(true);
  });

  it("maps a modulator onto a chain device's param: ring, depth chip, unmap", async () => {
    const rack = await rackWithSynth();
    fireEvent.click(within(rack).getByRole("button", { name: "Add modulator to Instrument Rack" }));
    const menu = useContextMenuStore.getState().menu!;
    const add = menu.items.find((i) => i !== "separator" && i.label === "Add LFO");
    if (!add || add === "separator") throw new Error("no Add LFO entry");
    add.onSelect();
    await flush();
    const lfo = await within(rack).findByRole("region", { name: "LFO" });
    fireEvent.click(within(lfo).getByRole("button", { name: "Map LFO" }));
    const delay = region("Delay");
    const targets = within(delay).getAllByTestId("mod-target");
    expect(targets.length).toBeGreaterThan(0);
    // The rack's macros are not targets of its own modulator.
    expect(within(rack).queryByRole("button", { name: /^Modulate Macro/ })).toBeNull();
    fireEvent.click(targets[0]!);
    await flush();
    const maps = Object.values(store().project!.mod_mappings);
    expect(maps).toHaveLength(1);
    expect(within(delay).queryAllByTestId("mod-target")).toHaveLength(0);
    expect(within(delay).getByTestId("mod-ring")).toBeInTheDocument();
    const chip = within(delay).getByTestId("mod-depth");
    expect(chip).toHaveAttribute("aria-valuetext", "+50 %");
    // The modulator lists its target with a depth knob; removing it unmaps.
    fireEvent.click(within(lfo).getByRole("button", { name: /^Remove modulation of Delay/ }));
    await flush();
    expect(Object.values(store().project!.mod_mappings)).toHaveLength(0);
  });

  it("uses macros as sources", async () => {
    const rack = await rackWithSynth();
    fireEvent.click(within(rack).getByRole("button", { name: "Map Macro 3" }));
    const delay = region("Delay");
    fireEvent.click(within(delay).getAllByTestId("mod-target")[1]!);
    await flush();
    const maps = Object.values(store().project!.mod_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]!.source).toMatchObject({ type: "Macro", index: 2 });
    await mock!.send(cmd("Edit", { type: "Undo" }));
    await flush();
    expect(Object.values(store().project!.mod_mappings)).toHaveLength(0);
  });
});
