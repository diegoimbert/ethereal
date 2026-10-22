/** A Drum Rack's macros (MockTransport): its window shows the 8 macro knobs above the pads,
 * each a modulation source; a pad device's params offer "assign to macro ›" with the drum
 * rack's macros (an outer instrument rack's too), one click maps them. */
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Command, ParamInfo } from "@/generated";
import { DeviceChain } from "@/features/devices";
import { DeviceWindowsRoot, resetDeviceWindows } from "@/features/devices/window";
import { flush, renderWithMock, resetStores, stubPointerCapture, store } from "@/features/devices/testUtils";
import { pickDevice } from "@/features/devices/testing";
import { paramMenuEntries, useModMapping } from "@/features/modulation";
import { cmd } from "@/transport";
import type { MockTransport } from "@/transport/mock";
import type { ContextMenuEntry } from "@/kit";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  cleanup();
  resetStores(mock);
  mock = undefined;
  resetDeviceWindows();
});

const PAD = "01DRMACPAD0000000000000000";
const SYNTH = "01DRMACSYNTH00000000000000";

async function drumRackWithSynthPad() {
  mock = await renderWithMock(
    <>
      <DeviceChain />
      <DeviceWindowsRoot />
    </>,
  );
  await screen.findByRole("region", { name: "Synth" });
  await pickDevice(screen.getByRole("button", { name: "add device" }), "drum rack");
  await flush();
  const rack = Object.values(store().project!.devices).find((d) => d.name === "drum rack")!;
  await act(async () => {
    await mock!.send(cmd("DrumRack", { type: "AddPad", id: PAD, rack: rack.id, note: 36, name: "Kick" }));
    await mock!.send(
      cmd("DrumRack", { type: "InsertDevice", id: SYNTH, pad: PAD, device: { type: "Builtin", device: { type: "Synth" } }, before: null }),
    );
  });
  await flush();
  return rack;
}

function labels(entries: readonly ContextMenuEntry[]): string[] {
  return entries.flatMap((e) => (e === "separator" ? [] : [e.label]));
}

describe("drum rack macros", () => {
  it("its window shows the 8 macro knobs, each a source mapped onto a pad device", async () => {
    const rack = await drumRackWithSynthPad();
    const card = screen.getByRole("region", { name: "drum rack" });
    const toggle = within(card).getByRole("button", { name: "open drum rack window" });
    if (toggle.getAttribute("aria-pressed") !== "true") fireEvent.click(toggle);
    const win = await screen.findByRole("dialog", { name: "drum rack window" });
    await waitFor(() => expect(within(win).getAllByRole("button", { name: /^map macro \d$/ })).toHaveLength(8));
    // Macro 2 is the drum rack's param 3 (after its volume and pan).
    expect(win.querySelector('[data-param="3"]')).not.toBeNull();
    // Its handle starts a mapping from macro 2 (index 1).
    fireEvent.click(within(win).getByRole("button", { name: "map macro 2" }));
    expect(useModMapping.getState().source).toEqual({ type: "Macro", rack: rack.id, index: 1 });
    act(() => useModMapping.getState().stop());
    const project = store().project!;
    const synth = project.devices[SYNTH]!;
    const info = { id: 0, name: "p", automatable: true } as ParamInfo;
    const sent: Command[] = [];
    const entries = paramMenuEntries(synth, info, (c) => {
      sent.push(c);
      return mock!.send(c);
    });
    const assign = entries.find((e) => e !== "separator" && e.label === "assign to macro");
    if (!assign || assign === "separator" || !assign.submenu) throw new Error("no assign to macro submenu");
    expect(labels(assign.submenu)).toEqual(Array.from({ length: 8 }, (_, i) => `macro ${i + 1}`));
    const macro4 = assign.submenu.find((e) => e !== "separator" && e.label === "macro 4");
    if (!macro4 || macro4 === "separator") throw new Error("no macro 4");
    await act(async () => macro4.onSelect());
    await flush();
    const maps = Object.values(store().project!.mod_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]).toMatchObject({ source: { type: "Macro", rack: rack.id, index: 3 }, device: SYNTH, param: 0 });
    // The rack's own macros are no target of its macros.
    const rackMenu = paramMenuEntries(project.devices[rack.id]!, { id: 2, name: "macro 1", automatable: true } as ParamInfo, () => {});
    expect(rackMenu).toEqual([]);
    expect(sent).toHaveLength(1);
  });

  it("an outer instrument rack's macros reach a nested drum rack's pads", async () => {
    await drumRackWithSynthPad();
    const p0 = store().project!;
    const drums = Object.values(p0.devices).find((d) => d.name === "drum rack")!;
    // Group the drum rack into an instrument rack.
    await act(async () => {
      await mock!.send(cmd("Rack", { type: "Group", rack: "01DRMACINST000000000000000", rack_type: "InstrumentRack", chain: "01DRMACCHAIN00000000000000", devices: [drums.id] }));
    });
    await flush();
    const p = store().project!;
    expect(p.devices[drums.id]!.chain).toBe("01DRMACCHAIN00000000000000");
    const entries = paramMenuEntries(p.devices[SYNTH]!, { id: 0, name: "p", automatable: true } as ParamInfo, () => {});
    const assign = entries.find((e) => e !== "separator" && e.label === "assign to macro");
    if (!assign || assign === "separator" || !assign.submenu) throw new Error("no assign to macro submenu");
    // Nearest first: the drum rack, then the instrument rack.
    expect(labels(assign.submenu)).toEqual(["drum rack", p.devices["01DRMACINST000000000000000"]!.name]);
  });
});
