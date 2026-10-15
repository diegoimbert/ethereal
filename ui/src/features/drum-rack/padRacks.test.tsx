import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { pickDevice } from "@/features/devices/testing";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { devicesOfPad, useSelectionStore } from "@/state";
import { cmd, type MockTransport } from "@/transport";
import { DrumRackView } from "./index";
import { noteName } from "./padUtils";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

const cell = (note: number) => screen.getByRole("gridcell", { name: new RegExp(`^${noteName(note)} `) });
const chainsOf = (rack: string) => Object.values(store().project!.rack_chains).filter((c) => c.rack === rack);

describe("racks on drum pads", () => {
  it("an Instrument Rack makes a pad, an Audio Effect Rack goes on its chain, both edit their chains", async () => {
    mock = await renderWithMock(<DrumRackView />);
    useSelectionStore.getState().selectTrack(trackByName("Keys").id);
    await flush();
    fireEvent.click(screen.getByRole("button", { name: "+ drum rack" }));
    await flush();

    // An empty note: the Instrument Rack creates the pad with it (one undo step).
    fireEvent.click(cell(36));
    await flush();
    await pickDevice(within(screen.getByTestId("pad-settings")).getAllByRole("button", { name: "add instrument" })[0]!, "instrument rack");
    await flush();
    await flush();
    const pad = Object.values(store().project!.drum_pads).find((p) => p.note === 36)!;
    expect(pad).toBeDefined();
    const [inst] = devicesOfPad(store().project!, pad.id);
    expect(inst?.kind).toMatchObject({ type: "Builtin", device: { type: "InstrumentRack" } });

    // An Audio Effect Rack after it.
    const settings = () => screen.getByTestId("pad-settings");
    await pickDevice(within(settings()).getAllByRole("button", { name: "add audio effect" })[0]!, "audio effect rack");
    await flush();
    const chain = devicesOfPad(store().project!, pad.id);
    expect(chain.map((d) => (d.kind.type === "Builtin" ? d.kind.device.type : null))).toEqual(["InstrumentRack", "AudioEffectRack"]);

    // Each rack card shows its chains: the instrument rack's "+ Add instrument" makes a chain
    // with the picked instrument; the effect rack came with its first (empty) chain.
    for (const rack of chain) {
      const panel = within(settings()).getByLabelText(`${rack.name} chains`);
      if (rack.kind.type === "Builtin" && rack.kind.device.type === "InstrumentRack") {
        fireEvent.click(
          within(panel)
            .getAllByRole("button", { name: /^add instrument$/i })
            .at(-1)!,
        );
        const library = await screen.findByRole("dialog", { name: "add instrument" });
        fireEvent.click(await within(library).findByRole("button", { name: "synth" }));
      }
      await flush();
      expect(chainsOf(rack.id)).toHaveLength(1);
    }

    // Undo: the instrument chain, the effect rack (with its first chain), then the pad with
    // its Instrument Rack at once.
    for (let i = 0; i < 2; i++) await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(devicesOfPad(store().project!, pad.id)).toHaveLength(1);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(store().project!.drum_pads[pad.id]).toBeUndefined();
  });
});
