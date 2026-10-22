/** A rack's macro bank (MockTransport): 8 of its 16 macros shown at first, the + / − buttons
 * show one more or hide the last one, double-click a macro's name (or "rename macro" in its
 * menu) renames it in place (one undo step), and the custom names show in "assign to macro ›",
 * which lists the shown macros only. */
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Device, ParamInfo } from "@/generated";
import { ContextMenuHost, type ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import type { MockTransport } from "@/transport/mock";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { useDescriptor } from "@/features/devices/descriptors";
import { useGestureSender } from "@/features/devices/gesture";
import { DeviceLayoutView } from "@/features/devices/layout";
import { useMacroRename } from "@/features/devices/layout/macros";
import { paramMenuEntries } from "@/features/modulation";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  cleanup();
  resetStores(mock);
  mock = undefined;
  act(() => useMacroRename.getState().stop());
});

const RACK = "01RKMACRACK000000000000000";
const CHAIN = "01RKMACCHAIN00000000000000";
const SYNTH = "01RKMACSYNTH00000000000000";

function RackPanel() {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[RACK]);
  return device ? <Loaded device={device} sender={sender} /> : null;
}

function Loaded({ device, sender }: { device: Device; sender: ReturnType<typeof useGestureSender> }) {
  const { descriptor } = useDescriptor(device);
  if (!descriptor) return null;
  return (
    <section aria-label="rack panel">
      <DeviceLayoutView device={device} descriptor={descriptor} sender={sender} />
    </section>
  );
}

async function rackWithSynth(): Promise<HTMLElement> {
  mock = await renderWithMock(
    <>
      <RackPanel />
      <ContextMenuHost />
    </>,
  );
  const track = trackByName("Keys").id;
  await act(async () => {
    await mock!.send(cmd("Device", { type: "Insert", id: RACK, track, device: { type: "Builtin", device: { type: "InstrumentRack" } }, before: null }));
    await mock!.send(cmd("Rack", { type: "AddChain", id: CHAIN, rack: RACK, name: null, before: null }));
    await mock!.send(cmd("Rack", { type: "InsertDevice", id: SYNTH, chain: CHAIN, device: { type: "Builtin", device: { type: "Synth" } }, before: null }));
  });
  await flush();
  return screen.findByRole("region", { name: "rack panel" });
}

const cells = (panel: HTMLElement) => Array.from(panel.querySelectorAll<HTMLElement>("[data-macro]"));
const rack = () => store().project!.devices[RACK]!;
const bank = () => {
  const k = rack().kind;
  return k.type === "Builtin" && "macros" in k.device ? (k.device.macros ?? null) : null;
};

function labels(entries: readonly ContextMenuEntry[]): string[] {
  return entries.flatMap((e) => (e === "separator" ? [] : [e.label]));
}

function assignSubmenu(device: Device): readonly ContextMenuEntry[] {
  const entries = paramMenuEntries(device, { id: 0, name: "p", automatable: true } as ParamInfo, () => {});
  const assign = entries.find((e) => e !== "separator" && e.label === "assign to macro");
  if (!assign || assign === "separator" || !assign.submenu) throw new Error("no assign to macro submenu");
  return assign.submenu;
}

describe("rack macro bank", () => {
  it("shows 8 macros, + / − show one more or hide the last one", async () => {
    const panel = await rackWithSynth();
    await waitFor(() => expect(cells(panel)).toHaveLength(8));
    expect(within(panel).getByRole("button", { name: "map macro 8" })).toBeDefined();
    fireEvent.click(within(panel).getByRole("button", { name: "show one more macro" }));
    await flush();
    expect(bank()).toEqual({ visible: 9, names: [] });
    await waitFor(() => expect(cells(panel)).toHaveLength(9));
    // Macro 9 is the rack's param 9 (after the chain selector).
    expect(cells(panel)[8]!.querySelector('[data-param="9"]')).not.toBeNull();
    // "assign to macro ›" lists the shown macros.
    expect(labels(assignSubmenu(store().project!.devices[SYNTH]!))).toEqual(Array.from({ length: 9 }, (_, i) => `macro ${i + 1}`));
    fireEvent.click(within(panel).getByRole("button", { name: "hide the last macro" }));
    await flush();
    fireEvent.click(within(panel).getByRole("button", { name: "hide the last macro" }));
    await flush();
    expect(bank()?.visible).toBe(7);
    await waitFor(() => expect(cells(panel)).toHaveLength(7));
  });

  it("double-click a name to rename the macro, one undo step; the name shows in the menus", async () => {
    const panel = await rackWithSynth();
    await waitFor(() => expect(cells(panel)).toHaveLength(8));
    const label = cells(panel)[2]!.querySelector(".eth-knob__label")!;
    fireEvent.doubleClick(label);
    const field = await within(panel).findByRole("textbox", { name: "name of macro 3" });
    fireEvent.change(field, { target: { value: "Bright" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(bank()).toEqual({ visible: 8, names: ["", "", "Bright"] });
    // The value wasn't reset by the double-click on the name.
    expect(rack().params[2] ?? 0).toBe(0);
    await waitFor(() => expect(cells(panel)[2]!.textContent).toContain("Bright"));
    expect(within(panel).getByRole("button", { name: "map Bright" })).toBeDefined();
    const sub = assignSubmenu(store().project!.devices[SYNTH]!);
    expect(labels(sub)[2]).toBe("Bright");
    const entry = sub[2];
    expect(entry !== "separator" && entry?.userText).toBe(true);

    // One undo step.
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    await flush();
    expect(bank()).toBeNull();

    // Escape cancels.
    fireEvent.doubleClick(cells(panel)[0]!.querySelector(".eth-knob__label")!);
    const again = await within(panel).findByRole("textbox", { name: "name of macro 1" });
    fireEvent.change(again, { target: { value: "Nope" } });
    fireEvent.keyDown(again, { key: "Escape" });
    await flush();
    expect(bank()).toBeNull();
  });

  it('"rename macro" in a macro knob\'s menu opens the name field', async () => {
    const panel = await rackWithSynth();
    await waitFor(() => expect(cells(panel)).toHaveLength(8));
    const entries = paramMenuEntries(rack(), { id: 4, name: "macro 5", automatable: true } as ParamInfo, () => {});
    expect(labels(entries)).toEqual(["rename macro"]);
    const rename = entries.find((e) => e !== "separator" && e.label === "rename macro");
    if (!rename || rename === "separator") throw new Error("no rename macro");
    act(() => rename.onSelect());
    const field = await within(panel).findByRole("textbox", { name: "name of macro 5" });
    fireEvent.change(field, { target: { value: "Space" } });
    fireEvent.blur(field);
    await flush();
    expect(bank()).toEqual({ visible: 8, names: ["", "", "", "", "Space"] });
    // The chain selector (not a macro) has no rename entry.
    expect(labels(paramMenuEntries(rack(), { id: 8, name: "chain select", automatable: true } as ParamInfo, () => {}))).not.toContain("rename macro");
  });
});
