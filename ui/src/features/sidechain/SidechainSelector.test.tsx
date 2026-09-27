import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Device, DeviceId } from "@/generated";
import { devicesOfTrack, useProjectStore } from "@/state";
import { cmd, newId, type MockTransport } from "@/transport";
import { flush, renderWithMock, resetStores, store, trackByName } from "@/features/mixer/testUtils";
import { SidechainSelector } from "./index";

let mock: MockTransport | undefined;
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

/** The selector for the live device `id` (re-renders on patches like DeviceView does). */
function Live({ id }: { id: DeviceId }) {
  const device = useProjectStore((s) => s.project?.devices[id]);
  return device ? <SidechainSelector device={device} /> : null;
}

const deviceOf = (track: string, name: string): Device =>
  devicesOfTrack(store().project!, trackByName(track).id).find((d) => d.name === name)!;

async function renderAll(ids: () => DeviceId[]) {
  mock = await renderWithMock(<Devices ids={ids} />);
}

function Devices({ ids }: { ids: () => DeviceId[] }) {
  // Subscribe so newly inserted devices render.
  useProjectStore((s) => s.project);
  if (!store().project) return null;
  return (
    <>
      {ids().map((id) => (
        <div key={id} data-testid={id}>
          <Live id={id} />
        </div>
      ))}
    </>
  );
}

const selector = (name: string) => screen.findByRole("combobox", { name: `Sidechain source for ${name}` });

describe("SidechainSelector", () => {
  it("renders only for devices with a sidechain input", async () => {
    await renderAll(() => devicesOfTrack(store().project!, trackByName("Keys").id).map((d) => d.id));
    const synth = deviceOf("Keys", "Synth");
    await selector("Compressor");
    await flush();
    expect(within(screen.getByTestId(synth.id)).queryByRole("combobox")).toBeNull();
  });

  it("lists tracks and returns (not master, not its own track) and sets / clears the source", async () => {
    await renderAll(() => [deviceOf("Keys", "Compressor").id]);
    const select = await selector("Compressor");
    const labels = within(select)
      .getAllByRole("option")
      .map((o) => o.textContent);
    const p = store().project!;
    const expected = Object.values(p.tracks).filter((t) => t.kind !== "Master" && t.name !== "Keys").length;
    expect(labels[0]).toBe("No sidechain");
    expect(labels).toHaveLength(expected + 1);
    expect(labels).toContain("Drums");
    expect(labels).not.toContain("Keys");
    expect(labels).not.toContain("Master");
    expect(select).toHaveDisplayValue("No sidechain");

    const drums = trackByName("Drums");
    await act(async () => {
      fireEvent.change(select, { target: { value: drums.id } });
    });
    await waitFor(() => expect(deviceOf("Keys", "Compressor").sidechain).toBe(drums.id));
    expect(await selector("Compressor")).toHaveDisplayValue("Drums");

    await act(async () => {
      fireEvent.change(select, { target: { value: "" } });
    });
    await waitFor(() => expect(deviceOf("Keys", "Compressor").sidechain).toBeNull());

    // One undo step per change.
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    await waitFor(() => expect(deviceOf("Keys", "Compressor").sidechain).toBe(drums.id));
  });

  it("disables sources that would close a routing cycle", async () => {
    const back = newId();
    await renderAll(() => (store().project!.devices[back] ? [deviceOf("Keys", "Compressor").id, back] : [deviceOf("Keys", "Compressor").id]));
    const drums = trackByName("Drums");
    const keys = trackByName("Keys");
    await act(async () => {
      await mock!.send(cmd("Device", { type: "SetSidechain", device: deviceOf("Keys", "Compressor").id, source: drums.id }));
      await mock!.send(
        cmd("Device", { type: "Insert", id: back, track: drums.id, device: { type: "Builtin", device: { type: "Limiter" } }, before: null }),
      );
    });
    const select = await selector("Limiter");
    const keysOption = within(select).getByRole("option", { name: `${keys.name} (would loop)` });
    expect(keysOption).toBeDisabled();
  });
});
