/** A Drum Rack as an instrument rack chain's instrument (MockTransport): the instrument rack's
 * "+ Add instrument" library offers it, a pick makes a chain holding it. */
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { DeviceChain } from "@/features/devices";
import { DeviceWindowsRoot, resetDeviceWindows } from "@/features/devices/window";
import { flush, renderWithMock, resetStores, stubPointerCapture, store } from "@/features/devices/testUtils";
import { pickDevice } from "@/features/devices/testing";
import type { MockTransport } from "@/transport/mock";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  cleanup();
  resetStores(mock);
  mock = undefined;
  resetDeviceWindows();
});

describe("Drum Rack on an instrument rack chain", () => {
  it("is offered by the instrument rack's instrument library and a pick inserts it on a new chain", async () => {
    mock = await renderWithMock(
      <>
        <DeviceChain />
        <DeviceWindowsRoot />
      </>,
    );
    await screen.findByRole("region", { name: "Synth" });
    await pickDevice(screen.getByRole("button", { name: "add device" }), "instrument rack");
    await flush();
    const rack = await screen.findByRole("region", { name: "instrument rack" });
    fireEvent.click(
      within(rack)
        .getAllByRole("button", { name: /^add instrument$/i })
        .at(-1)!,
    );
    const library = await screen.findByRole("dialog", { name: "add instrument" });
    const list = await within(library).findByRole("list");
    await waitFor(() => expect(list).not.toHaveAttribute("aria-busy", "true"));
    const offered = within(list)
      .queryAllByRole("listitem")
      .map((li) => li.querySelector(".eth-devlib__name")?.textContent ?? "");
    expect(offered).toContain("drum rack");
    fireEvent.click(await within(library).findByRole("button", { name: "drum rack" }));
    await flush();
    await flush();
    const p = store().project!;
    const inst = Object.values(p.devices).find((d) => d.name === "instrument rack")!;
    const chains = Object.values(p.rack_chains).filter((c) => c.rack === inst.id);
    expect(chains).toHaveLength(1);
    const drums = Object.values(p.devices).filter((d) => d.chain === chains[0]!.id);
    expect(drums.map((d) => (d.kind.type === "Builtin" ? d.kind.device.type : null))).toEqual(["DrumRack"]);
    expect(drums[0]!.name).toBe("drum rack");
  });
});
