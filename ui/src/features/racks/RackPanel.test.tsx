/** Racks and modulation in the device chain (MockTransport): chains, chain devices, a
 * modulator mapped onto a chain device's knob (depth ring + chip), macros as sources. */
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { DeviceChain } from "@/features/devices";
import { DeviceWindowsRoot, resetDeviceWindows } from "@/features/devices/window";
import { flush, renderWithMock, resetStores, stubPointerCapture, store } from "@/features/devices/testUtils";
import { useContextMenuStore } from "@/kit";
import { pickDevice } from "@/features/devices/testing";
import { cmd } from "@/transport";
import { type MockTransport } from "@/transport/mock";

/** Names of the rows the open picker offers (its list has loaded). */
async function offeredDevices(trigger: HTMLElement): Promise<string[]> {
  const popover = await screen.findByRole("dialog", { name: trigger.getAttribute("aria-label")! });
  const list = await within(popover).findByRole("list");
  await waitFor(() => expect(list).not.toHaveAttribute("aria-busy", "true"));
  return within(list)
    .queryAllByRole("listitem")
    .map((li) => li.querySelector(".eth-devlib__name")?.textContent ?? "");
}

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  cleanup();
  resetStores(mock);
  mock = undefined;
  resetDeviceWindows();
});

const region = (name: string) => screen.getByRole("region", { name });

/** A device's own UI (layout, macros, modulation targets) is in its window. */
async function openWindow(name: string): Promise<HTMLElement> {
  // A device just added from the menu has its window open already.
  const toggle = within(region(name)).getByRole("button", { name: `open ${name} window` });
  if (toggle.getAttribute("aria-pressed") !== "true") fireEvent.click(toggle);
  const win = await screen.findByRole("dialog", { name: `${name} window` });
  await waitFor(() => expect(within(win).queryByText("loading…")).toBeNull());
  return win;
}

/** A chain rack has no window: its macros are on its card, behind the parameters toggle. */
async function openParams(name: string): Promise<HTMLElement> {
  const card = region(name);
  const toggle = within(card).getByRole("button", { name: `show ${name} parameters` });
  if (toggle.getAttribute("aria-pressed") !== "true") fireEvent.click(toggle);
  await flush();
  return card;
}

const mockOf = (): MockTransport => {
  if (!mock) throw new Error("not rendered");
  return mock;
};

async function renderChain() {
  mock = await renderWithMock(
    <>
      <DeviceChain />
      <DeviceWindowsRoot />
    </>,
  );
  await screen.findByRole("region", { name: "Synth" });
}

/** "+ Add instrument" on an instrument rack: pick `name` in its library. */
async function addInstrument(rack: HTMLElement, name: string) {
  const add = within(rack)
    .getAllByRole("button", { name: /^add instrument$/i })
    .at(-1)!;
  fireEvent.click(add);
  const library = await screen.findByRole("dialog", { name: "add instrument" });
  fireEvent.click(await within(library).findByRole("button", { name }));
  await flush();
}

async function rackWithSynth() {
  await renderChain();
  await pickDevice(screen.getByRole("button", { name: "add device" }), "instrument rack");
  await flush();
  const rack = await screen.findByRole("region", { name: "instrument rack" });
  // An instrument rack adds instruments: "+ Add instrument" opens the library, a pick makes a
  // chain with it (one undo step), collapsed.
  expect(within(rack).queryByRole("button", { name: "add chain" })).toBeNull();
  await addInstrument(rack, "synth");
  expect(within(rack).getByRole("button", { name: "chain 1 chain" })).toHaveAttribute("aria-expanded", "false");
  // Unnamed, the chain shows its instrument's name.
  expect(rack.querySelector(".eth-rack__name")).toHaveTextContent("synth");
  // Expanded, with its instrument, the chain shows every section (MIDI / instrument / audio).
  fireEvent.click(within(rack).getByRole("button", { name: "chain 1 chain" }));
  await flush();
  await pickDevice(within(rack).getAllByRole("button", { name: "add audio effect" })[0]!, "delay");
  await flush();
  return rack;
}

describe("RackPanel", () => {
  it("adds chains and chain devices, which stay off the track chain", async () => {
    const rack = await rackWithSynth();
    const delay = await within(rack).findByRole("region", { name: "delay" });
    expect(delay).toBeInTheDocument();
    const p = store().project!;
    const d = Object.values(p.devices).find((x) => x.name === "delay" && x.chain != null);
    expect(d).toBeDefined();
    // The track chain list shows the rack only once, not its chain devices (the rack, an
    // instrument, replaced the Synth: rack + Compressor).
    const list = screen.getByRole("list", { name: "Keys devices" });
    expect(
      within(list)
        .getAllByRole("region")
        .filter((r) => r.closest('[role="list"]') === list),
    ).toHaveLength(2);
    // Rename in place: double-click the name, type, Enter.
    fireEvent.doubleClick(rack.querySelector(".eth-rack__name")!);
    const field = within(rack).getByRole("textbox", { name: "chain name" });
    fireEvent.change(field, { target: { value: "Pads" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(Object.values(store().project!.rack_chains)[0]!.name).toBe("Pads");
    fireEvent.doubleClick(rack.querySelector(".eth-rack__name")!);
    // Emptied: back to its default name.
    fireEvent.change(within(rack).getByRole("textbox", { name: "chain name" }), { target: { value: "  " } });
    fireEvent.keyDown(within(rack).getByRole("textbox", { name: "chain name" }), { key: "Enter" });
    await flush();
    expect(Object.values(store().project!.rack_chains)[0]!.name).toBe("chain 1");
    // Mute and solo.
    fireEvent.click(within(rack).getByRole("button", { name: "mute chain 1" }));
    await flush();
    expect(Object.values(store().project!.rack_chains)[0]!.mute).toBe(true);
  });

  it("exclusive chain solo: plain click solos only that chain, Cmd/Ctrl adds, the only soloed one unsolos", async () => {
    const rack = await rackWithSynth();
    // Device windows only (not the instrument library popovers).
    const deviceWindows = () => screen.queryAllByRole("dialog").filter((d) => d.classList.contains("eth-fwin")).length;
    for (let i = 0; i < 2; i++) await addInstrument(rack, "synth");
    // (Each added instrument opened its window; the solo clicks below must open none.)
    const windows = deviceWindows();
    const soloed = () =>
      Object.values(store().project!.rack_chains)
        .filter((c) => c.solo)
        .map((c) => c.name)
        .sort();
    const click = async (name: string, mods: { metaKey?: boolean; ctrlKey?: boolean } = {}) => {
      const btn = within(rack).getByRole("button", { name: new RegExp(`^(un)?[sS]olo ${name}$`) });
      fireEvent.click(btn, mods);
      await flush();
    };
    const names = Object.values(store().project!.rack_chains)
      .map((c) => c.name)
      .sort();
    expect(names).toHaveLength(3);
    const [a, b, c] = names as [string, string, string];
    await click(a, { metaKey: true });
    await click(b, { ctrlKey: true });
    await click(c, { metaKey: true });
    expect(soloed()).toEqual([a, b, c]);
    // Plain click on a soloed chain: only it stays.
    await click(b);
    expect(soloed()).toEqual([b]);
    // Plain click on an unsoloed chain: only it.
    await click(c);
    expect(soloed()).toEqual([c]);
    // Plain click on the only soloed chain: none.
    await click(c);
    expect(soloed()).toEqual([]);
    // One undo step per click (the exclusive one reverts as a whole).
    await click(a, { metaKey: true });
    await click(b, { metaKey: true });
    await click(c);
    expect(soloed()).toEqual([c]);
    await act(() => mockOf().send(cmd("Edit", { type: "Undo" })));
    expect(soloed()).toEqual([a, b]);
    // Clicking solo does not open another device window.
    expect(deviceWindows()).toBe(windows);
  });

  it("maps a modulator onto a chain device's param: ring, depth chip, unmap", async () => {
    const rack = await rackWithSynth();
    fireEvent.click(within(rack).getByRole("button", { name: "add modulator to instrument rack" }));
    const menu = useContextMenuStore.getState().menu!;
    const add = menu.items.find((i) => i !== "separator" && i.label === "add lfo");
    if (!add || add === "separator") throw new Error("no add lfo entry");
    add.onSelect();
    await flush();
    const lfo = await within(rack).findByRole("region", { name: "lfo" });
    const delay = await openWindow("delay");
    const rackWindow = await openParams("instrument rack");
    fireEvent.click(within(lfo).getByRole("button", { name: "map lfo" }));
    const targets = within(delay).getAllByTestId("mod-target");
    expect(targets.length).toBeGreaterThan(0);
    // The rack's macros are not targets of its own modulator.
    expect(within(rackWindow).queryByRole("button", { name: /^modulate macro/ })).toBeNull();
    fireEvent.click(targets[0]!);
    await flush();
    const maps = Object.values(store().project!.mod_mappings);
    expect(maps).toHaveLength(1);
    expect(within(delay).queryAllByTestId("mod-target")).toHaveLength(0);
    expect(within(delay).getByTestId("mod-ring")).toBeInTheDocument();
    const chip = within(delay).getByTestId("mod-depth");
    expect(chip).toHaveAttribute("aria-valuetext", "+50 %");
    // The decoration never touches the label: the knob keeps its name, the value shows
    // once, and the ring/chips come after the knob (chips below it).
    const shell = chip.closest("[data-param]")!;
    const knob = shell.querySelector(".eth-knob")!;
    const name = knob.querySelector(".eth-knob__name")!.textContent!;
    expect(name).not.toBe("");
    expect(knob.getAttribute("aria-label")).toBe(name);
    // The value: under the knob (sm/md) or in its center (lg), exactly once.
    expect(shell.querySelectorAll(".eth-param__value, .eth-knob__center")).toHaveLength(1);
    expect(shell.querySelector(".eth-mod-chips")!.textContent).toBe("");
    expect(knob.compareDocumentPosition(chip) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // The modulator lists its target with a depth knob; removing it unmaps.
    fireEvent.click(within(lfo).getByRole("button", { name: /^remove modulation of delay/ }));
    await flush();
    expect(Object.values(store().project!.mod_mappings)).toHaveLength(0);
  });

  it("adds a rack inside a chain, drawn one level in with its own chains", async () => {
    const rack = await rackWithSynth();
    await pickDevice(within(rack).getAllByRole("button", { name: "add audio effect" })[0]!, "audio effect rack");
    await flush();
    const nested = await within(rack).findByRole("region", { name: "audio effect rack" });
    const p = store().project!;
    const nestedRack = Object.values(p.devices).find((d) => d.name === "audio effect rack")!;
    expect(p.rack_chains[nestedRack.chain!]?.rack).toBe(Object.values(p.devices).find((d) => d.name === "instrument rack")!.id);
    // It comes with its first chain (collapsed: open it to add effects).
    expect(Object.values(p.rack_chains).filter((c) => c.rack === nestedRack.id)).toHaveLength(1);
    expect(nested).toBeInTheDocument();
    const panel = nested.querySelector<HTMLElement>(`.eth-rack[data-rack="${nestedRack.id}"]`)!;
    expect(panel).toHaveClass("eth-rack--nested");
    expect(panel.dataset.level).toBe("2");
    fireEvent.click(within(panel).getByRole("button", { name: "chain 1 chain" }));
    await flush();
    // An audio effect rack's chain holds audio effects only (its one section), racks of that
    // kind included.
    expect(within(panel).queryByRole("button", { name: /^add instrument$/i })).toBeNull();
    const picker = within(panel).getAllByRole("button", { name: "add audio effect" })[0]!;
    fireEvent.click(picker);
    const offered = await offeredDevices(picker);
    expect(offered).toContain("audio effect rack");
    expect(offered).not.toContain("instrument rack");
    expect(offered).not.toContain("drum rack");
    // The same search picker as everywhere.
    expect(screen.getByRole("searchbox", { name: /^search/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "reverb" }));
    await flush();
    expect(within(panel).getByRole("region", { name: "reverb" })).toBeInTheDocument();
  });

  it("offers no racks on the deepest chains", async () => {
    await renderChain();
    const mock = mockOf();
    const track = Object.values(store().project!.tracks).find((t) => t.name === "Keys")!.id;
    const ids = Array.from({ length: 12 }, () => crypto.randomUUID());
    await mock.send(
      cmd("Device", { type: "Insert", id: ids[0]!, track, device: { type: "Builtin", device: { type: "AudioEffectRack" } as never }, before: null }),
    );
    for (let level = 1; level <= 6; level++) {
      const rack = ids[(level - 1) * 2]!;
      // Each effect rack comes with its first chain.
      const chain = Object.values(mock.snapshot().rack_chains).find((c) => c.rack === rack)!.id;
      await mock.send(cmd("Rack", { type: "RenameChain", id: chain, name: `L${level}` }));
      if (level < 6) {
        const next = ids[level * 2]!;
        await mock.send(
          cmd("Rack", { type: "InsertDevice", id: next, chain, device: { type: "Builtin", device: { type: "AudioEffectRack" } as never }, before: null }),
        );
      }
    }
    await flush();
    // Chains open one level at a time (collapsed by default).
    for (let level = 1; level <= 6; level++) {
      fireEvent.click(screen.getByRole("button", { name: `L${level} chain` }));
      await flush();
    }
    const deepest = document.querySelector<HTMLElement>(`.eth-rack[data-rack="${ids[10]}"]`)!;
    expect(deepest.dataset.level).toBe("6");
    const picker = within(deepest).getAllByRole("button", { name: "add audio effect" })[0]!;
    fireEvent.click(picker);
    const offered = await offeredDevices(picker);
    expect(offered).toContain("reverb");
    expect(offered).not.toContain("audio effect rack");
  });

  it("a param inside a rack: its context menu assigns it to one of the rack's macros", async () => {
    await rackWithSynth();
    const delay = await openWindow("delay");
    const param = delay.querySelector<HTMLElement>("[data-param]")!;
    fireEvent.contextMenu(param);
    const items = useContextMenuStore.getState().menu!.items;
    const assign = items.find((i) => i !== "separator" && i.label === "assign to macro");
    if (!assign || assign === "separator" || !assign.submenu) throw new Error("no assign to macro submenu");
    const macro3 = assign.submenu.find((i) => i !== "separator" && i.label === "macro 3");
    if (!macro3 || macro3 === "separator") throw new Error("no macro 3");
    act(() => macro3.onSelect());
    await flush();
    const maps = Object.values(store().project!.mod_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]!.source).toMatchObject({ type: "Macro", index: 2 });
    expect(maps[0]!.param).toBe(Number(param.dataset.param));
    // Assigned: that macro is disabled next time.
    fireEvent.contextMenu(param);
    const again = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "assign to macro");
    const sub = again !== undefined && again !== "separator" ? again.submenu! : [];
    expect(sub.find((i) => i !== "separator" && i.label === "macro 3")).toMatchObject({ disabled: true });
  });

  it("uses macros as sources", async () => {
    await rackWithSynth();
    const delay = await openWindow("delay");
    const rackWindow = await openParams("instrument rack");
    fireEvent.click(within(rackWindow).getByRole("button", { name: "map macro 3" }));
    fireEvent.click(within(delay).getAllByTestId("mod-target")[1]!);
    await flush();
    const maps = Object.values(store().project!.mod_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]!.source).toMatchObject({ type: "Macro", index: 2 });
    await mock!.send(cmd("Edit", { type: "Undo" }));
    await flush();
    expect(Object.values(store().project!.mod_mappings)).toHaveLength(0);
  });

  it("a macro switches a chain device on and off: assigned from its power button, shown live", async () => {
    await rackWithSynth();
    const power = () => region("delay").querySelector<HTMLElement>(".eth-device__power")!;
    const delayId = () => Object.values(store().project!.devices).find((d) => d.name === "delay")!.id;
    // The power button's menu: "assign to macro › macro 1" (the delay is on: the macro turns
    // it off from 0.5).
    fireEvent.contextMenu(power());
    const assign = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "assign to macro");
    if (!assign || assign === "separator" || !assign.submenu) throw new Error("no assign to macro submenu");
    const macro1 = assign.submenu.find((i) => i !== "separator" && i.label === "macro 1");
    if (!macro1 || macro1 === "separator") throw new Error("no macro 1");
    act(() => macro1.onSelect());
    await flush();
    const maps = Object.values(store().project!.mod_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]).toMatchObject({ source: { type: "Macro", index: 0 }, device: delayId(), param: 0xffff_ffff, depth: -1 });
    expect(within(region("delay")).getByTestId("mod-switch")).toBeInTheDocument();
    expect(power()).toHaveAttribute("data-modulated", "true");
    expect(power().title).toContain("switched by a macro or modulator");
    // The engine's readback: the macro switched it off.
    const events = (mock as unknown as { events: { emit(e: unknown): void } }).events;
    act(() =>
      events.emit({
        type: "Analysis",
        event: { type: "Frame", device: delayId(), data: { type: "Modulation", values: [{ param: 0xffff_ffff, base: 1, value: 0 }] } },
      }),
    );
    await waitFor(() => expect(power()).toHaveAttribute("aria-pressed", "false"));
    expect(region("delay")).toHaveClass("eth-device--bypassed");
    // Undo removes it.
    await mock!.send(cmd("Edit", { type: "Undo" }));
    await flush();
    expect(Object.values(store().project!.mod_mappings)).toHaveLength(0);
    expect(power()).toHaveAttribute("aria-pressed", "true");
    // Mapping mode: the chain device's switch is a target, the rack's own isn't.
    const rackCard = await openParams("instrument rack");
    fireEvent.click(within(rackCard).getByRole("button", { name: "map macro 3" }));
    await flush();
    expect(rackCard.querySelector(":scope > header .eth-mod-target")).toBeNull();
    fireEvent.click(within(region("delay")).getByRole("button", { name: "switch delay on and off" }));
    await flush();
    expect(Object.values(store().project!.mod_mappings)).toEqual([
      expect.objectContaining({ source: expect.objectContaining({ type: "Macro", index: 2 }), param: 0xffff_ffff, depth: -1 }),
    ]);
  });
});
