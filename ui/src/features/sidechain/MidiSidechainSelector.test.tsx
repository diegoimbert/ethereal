import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Device, DeviceId } from "@/generated";
import { devicesOfTrack, useProjectStore } from "@/state";
import { cmd, newId } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { flush, renderWithMock, resetStores, store, trackByName } from "@/features/devices/testUtils";
import { MidiSidechainSelector } from "./index";

let mock: MockTransport | undefined;
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

function Live({ id }: { id: DeviceId }) {
  const device = useProjectStore((s) => s.project?.devices[id]);
  return device ? <MidiSidechainSelector device={device} /> : null;
}

function Devices({ ids }: { ids: () => DeviceId[] }) {
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

const deviceOf = (track: string, name: string): Device => devicesOfTrack(store().project!, trackByName(track).id).find((d) => d.name === name)!;
const selector = (name: string) => screen.findByRole("button", { name: `midi input for ${name}` });
async function sources(name: string) {
  const button = await selector(name);
  if (button.getAttribute("aria-expanded") !== "true") fireEvent.click(button);
  return within(await screen.findByRole("menu", { name: "midi input sources" }));
}

/** Keys gets a Shifter (an audio effect with a note input) after its synth. */
async function withShifter(): Promise<DeviceId> {
  const id = newId();
  mock = await renderWithMock(<Devices ids={() => (store().project!.devices[id] ? [id, deviceOf("Keys", "Compressor").id] : [])} />);
  await act(async () => {
    await mock!.send(
      cmd("Device", {
        type: "Insert",
        id,
        track: trackByName("Keys").id,
        device: { type: "Builtin", device: { type: "Shifter" } },
        before: null,
      }),
    );
  });
  return id;
}

describe("MidiSidechainSelector", () => {
  it("renders only for audio effects with a note input", async () => {
    await withShifter();
    await selector("shifter");
    await flush();
    const comp = deviceOf("Keys", "Compressor");
    expect(within(screen.getByTestId(comp.id)).queryByRole("button", { name: /midi input/ })).toBeNull();
  });

  it("lists the midi tracks (its own too), sets pre or post fx and clears in one undo step each", async () => {
    const id = await withShifter();
    const p = store().project!;
    const list = await sources("shifter");
    const labels = list.getAllByRole("menuitem").map((o) => o.textContent);
    expect(labels[0]).toBe("no midi input");
    expect(labels).toHaveLength(Object.values(p.tracks).filter((t) => t.kind === "Midi").length + 1);
    expect(labels).toContain("Keys (this track)");
    const keys = trackByName("Keys");

    // A click on a track: post fx.
    fireEvent.click(list.getByRole("menuitem", { name: "Keys (this track)" }));
    await waitFor(() => expect(store().project!.devices[id]!.midi_sidechain).toEqual({ track: keys.id, tap: "PostFx" }));
    expect(await selector("shifter")).toHaveTextContent("Keys (this track) · post fx");

    // The submenu: pre fx.
    const again = await sources("shifter");
    fireEvent.mouseEnter(again.getByRole("menuitem", { name: "Keys (this track)" }));
    fireEvent.click(within(screen.getByRole("menu", { name: "Keys (this track)" })).getByRole("menuitem", { name: "pre fx" }));
    await waitFor(() => expect(store().project!.devices[id]!.midi_sidechain?.tap).toBe("PreFx"));

    fireEvent.click((await sources("shifter")).getByRole("menuitem", { name: "no midi input" }));
    await waitFor(() => expect(store().project!.devices[id]!.midi_sidechain).toBeUndefined());
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    await waitFor(() => expect(store().project!.devices[id]!.midi_sidechain).toEqual({ track: keys.id, tap: "PreFx" }));
  });
});
