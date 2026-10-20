import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { pickOption } from "@/kit/testing";
import { afterEach, describe, expect, it } from "vitest";
import type { Device, DeviceId } from "@/generated";
import { devicesOfTrack, useProjectStore } from "@/state";
import { cmd, deriveId, newId } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { flush, renderWithMock, resetStores, store, trackByName } from "@/features/devices/testUtils";
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

const deviceOf = (track: string, name: string): Device => devicesOfTrack(store().project!, trackByName(track).id).find((d) => d.name === name)!;

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

const selector = (name: string) => screen.findByRole("combobox", { name: `sidechain source for ${name}` });
const tapSelector = (name: string) => screen.findByRole("combobox", { name: `sidechain tap for ${name}` });
/** The options of a kit Select (opens it; they are only rendered while open). */
const optionsOf = (select: HTMLElement) => {
  if (select.getAttribute("aria-expanded") !== "true") fireEvent.click(select);
  return within(document.getElementById(select.getAttribute("aria-controls")!)!);
};

describe("SidechainSelector", () => {
  it("renders only for devices with a sidechain input", async () => {
    await renderAll(() => devicesOfTrack(store().project!, trackByName("Keys").id).map((d) => d.id));
    const synth = deviceOf("Keys", "Synth");
    await selector("Compressor");
    await flush();
    expect(within(screen.getByTestId(synth.id)).queryByRole("combobox")).toBeNull();
  });

  it("lists tracks and returns (not master; its own track for its pre-FX signal) and sets / clears the source", async () => {
    await renderAll(() => [deviceOf("Keys", "Compressor").id]);
    const select = await selector("Compressor");
    const labels = optionsOf(select)
      .getAllByRole("option")
      .map((o) => o.textContent);
    const p = store().project!;
    const expected = Object.values(p.tracks).filter((t) => t.kind !== "Master").length;
    expect(labels[0]).toBe("no sidechain");
    expect(labels).toHaveLength(expected + 1);
    expect(labels).toContain("Drums");
    expect(labels).toContain("Keys (this track)");
    expect(labels).not.toContain("master");
    fireEvent.keyDown(select, { key: "Escape" });
    expect(select).toHaveTextContent("no sidechain");

    const drums = trackByName("Drums");
    await act(async () => {
      pickOption(select, { value: drums.id });
    });
    await waitFor(() => expect(deviceOf("Keys", "Compressor").sidechain).toBe(drums.id));
    expect(await selector("Compressor")).toHaveTextContent("Drums");
    // The tap picker appears with the source: post-fader ("post mixer") by default.
    expect(await tapSelector("Compressor")).toHaveTextContent("post mixer");

    const current = await selector("Compressor");
    await act(async () => {
      pickOption(current, { value: "" });
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
      await mock!.send(
        cmd("Device", {
          type: "SetSidechain",
          device: deviceOf("Keys", "Compressor").id,
          source: drums.id,
        }),
      );
      await mock!.send(
        cmd("Device", {
          type: "Insert",
          id: back,
          track: drums.id,
          device: { type: "Builtin", device: { type: "Limiter" } },
          before: null,
        }),
      );
    });
    const select = await selector("limiter");
    const keysOption = optionsOf(select).getByRole("option", {
      name: `${keys.name} (would loop)`,
    });
    expect(keysOption).toHaveAttribute("aria-disabled", "true");
  });

  it("keys a device two racks deep from a rack chain of another track", async () => {
    const [rack, inner, comp, keysRack, keysChain, synth] = Array.from({ length: 6 }, () => newId()) as [string, string, string, string, string, string];
    // Audio effect racks come with their first chain (id derived from the rack's).
    const [chain, innerChain] = [deriveId(rack, 0), deriveId(inner, 0)];
    await renderAll(() => (store().project!.devices[comp] ? [comp] : []));
    const bass = trackByName("Bass");
    const keys = trackByName("Keys");
    const builtin = (type: string) => ({ type: "Builtin", device: { type } }) as never;
    await act(async () => {
      // Bass: Audio Effect Rack → "Wet" → Audio Effect Rack → chain → Compressor.
      await mock!.send(
        cmd("Device", {
          type: "Insert",
          id: rack,
          track: bass.id,
          device: builtin("AudioEffectRack"),
          before: null,
        }),
      );
      await mock!.send(
        cmd("Rack", {
          type: "RenameChain",
          id: chain,
          name: "Wet",
        }),
      );
      await mock!.send(
        cmd("Rack", {
          type: "InsertDevice",
          id: inner,
          chain,
          device: builtin("AudioEffectRack"),
          before: null,
        }),
      );
      await mock!.send(
        cmd("Rack", {
          type: "RenameChain",
          id: innerChain,
          name: "Duck",
        }),
      );
      await mock!.send(
        cmd("Rack", {
          type: "InsertDevice",
          id: comp,
          chain: innerChain,
          device: builtin("Compressor"),
          before: null,
        }),
      );
      // Keys: Instrument Rack → "Kick SC" → Synth.
      await mock!.send(
        cmd("Device", {
          type: "Insert",
          id: keysRack,
          track: keys.id,
          device: builtin("InstrumentRack"),
          before: null,
        }),
      );
      await mock!.send(
        cmd("Rack", {
          type: "AddChain",
          id: keysChain,
          rack: keysRack,
          name: "Kick SC",
          before: null,
        }),
      );
      await mock!.send(
        cmd("Rack", {
          type: "InsertDevice",
          id: synth,
          chain: keysChain,
          device: builtin("Synth"),
          before: null,
        }),
      );
    });
    // The nested compressor has its picker.
    await flush();
    const select = await selector("compressor");
    pickOption(select, { value: keys.id });
    await waitFor(() => expect(store().project!.devices[comp]!.sidechain).toBe(keys.id));
    const tap = await tapSelector("compressor");
    const labels = optionsOf(tap)
      .getAllByRole("option")
      .map((o) => o.textContent);
    expect(labels).toContain("instrument rack | Kick SC | pre fx");
    fireEvent.keyDown(tap, { key: "Escape" });
    pickOption(tap, { value: `PreFx|R|${keysChain}` });
    await waitFor(() =>
      expect(store().project!.devices[comp]!.sidechain_tap).toEqual({
        point: "PreFx",
        chain: { type: "Rack", chain: keysChain },
      }),
    );
    // Its own track: only points heard before it (pre FX), not its own racks' chains.
    pickOption(await selector("compressor"), { value: bass.id });
    await waitFor(() => expect(store().project!.devices[comp]!.sidechain).toBe(bass.id));
    expect(store().project!.devices[comp]!.sidechain_tap).toEqual({
      point: "PreFx",
    });
    const ownSelect = await tapSelector("compressor");
    const own = optionsOf(ownSelect);
    expect(own.getByRole("option", { name: "post fx (comes after this device)" })).toHaveAttribute("aria-disabled", "true");
    expect(
      own.getByRole("option", {
        name: "audio effect rack | Wet | post fx (holds this device)",
      }),
    ).toHaveAttribute("aria-disabled", "true");
    fireEvent.keyDown(ownSelect, { key: "Escape" });
    // Removing the tapped chain cuts the sidechain; undo brings both back.
    pickOption(await selector("compressor"), { value: keys.id });
    await waitFor(() => expect(store().project!.devices[comp]!.sidechain).toBe(keys.id));
    pickOption(await tapSelector("compressor"), {
      value: `PostFx|R|${keysChain}`,
    });
    await waitFor(() => expect(store().project!.devices[comp]!.sidechain_tap?.point).toBe("PostFx"));
    await act(async () => {
      await mock!.send(cmd("Rack", { type: "RemoveChain", id: keysChain }));
    });
    await waitFor(() => expect(store().project!.devices[comp]!.sidechain).toBeNull());
    expect(store().project!.devices[comp]!.sidechain_tap).toBeUndefined();
  });
});
