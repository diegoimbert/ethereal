import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { DeviceId, ExternalRouting, HardwarePorts } from "@/generated";
import { useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, cmd, newId, type MockTransport } from "@/transport";
import { flush, renderWithMock, resetStores, store, trackByName } from "@/features/mixer/testUtils";
import { useGestureSender } from "@/features/devices/gesture";
import { LayoutContext, useLayoutContextValue } from "@/features/devices/layout/context";
import { HardwareRoutingWidget } from "./HardwareRouting";
import { canMeasure, channelOptions, midiOptions, missingParts, parseChannels } from "./routing";

let mock: MockTransport | undefined;
let id: DeviceId = "";
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

const PORTS: HardwarePorts = {
  midi_outputs: [{ id: "Synth A", name: "Synth A" }],
  audio_inputs: [0, 1, 2].map((index) => ({ index, name: `In ${index + 1}` })),
  audio_outputs: [0, 1, 2, 3].map((index) => ({ index, name: `Out ${index + 1}` })),
};
const EMPTY: ExternalRouting = { midi_out: null, midi_channel: 1, audio_send: null, audio_return: null };

describe("routing helpers", () => {
  it("offers stereo pairs and mono channels, keeping a missing current value", () => {
    const opts = channelOptions(PORTS.audio_outputs, null, "Out", "missing");
    expect(opts.map((o) => o.label)).toEqual(["None", "Out 1/2", "Out 3/4", "Out 1", "Out 2", "Out 3", "Out 4"]);
    expect(parseChannels(opts[2]!.value)).toEqual({ first: 2, count: 2 });
    // Three inputs: one pair only.
    expect(channelOptions(PORTS.audio_inputs, null, "In", "x").filter((o) => o.group === "Stereo")).toHaveLength(1);
    const kept = channelOptions(PORTS.audio_inputs, { first: 6, count: 2 }, "In", "missing");
    expect(kept.at(-1)).toEqual({ value: "6:2", label: "In 7/8 (missing)" });
    expect(midiOptions(PORTS, "Gone", "missing").map((o) => o.label)).toEqual(["None", "Synth A", "Gone (missing)"]);
  });

  it("finds missing ports and what a measurement needs", () => {
    const r: ExternalRouting = { midi_out: "Gone", midi_channel: 1, audio_send: { first: 2, count: 2 }, audio_return: { first: 2, count: 2 } };
    expect(missingParts(r, PORTS)).toEqual(['MIDI "Gone"', "audio return"]);
    expect(missingParts(r, null)).toEqual([]);
    expect(canMeasure("ExternalInstrument", { ...EMPTY, midi_out: "Synth A" })).toBe(false);
    expect(canMeasure("ExternalInstrument", { ...EMPTY, midi_out: "Synth A", audio_return: { first: 0, count: 1 } })).toBe(true);
    expect(canMeasure("ExternalAudioEffect", { ...EMPTY, audio_return: { first: 0, count: 1 } })).toBe(false);
  });
});

function Panel() {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[id]);
  const type = device?.kind.type === "Builtin" ? device.kind.device.type : "ExternalInstrument";
  const ctx = useLayoutContextValue(device!, BUILTIN_DESCRIPTORS[type], sender);
  if (!device) return null;
  return (
    <LayoutContext.Provider value={ctx}>
      <HardwareRoutingWidget widget={{ type: "HardwareRouting" }} size="Large" label={null} />
    </LayoutContext.Provider>
  );
}

async function render(type: "ExternalInstrument" | "ExternalAudioEffect") {
  id = newId();
  mock = await renderWithMock(<Panel />);
  await act(async () => {
    await mock!.send(
      cmd("Device", {
        type: "Insert",
        id,
        track: trackByName(type === "ExternalInstrument" ? "Keys" : "Drums").id,
        device: { type: "Builtin", device: { type, routing: EMPTY } },
        before: null,
      }),
    );
  });
  await flush();
  return screen.getByTestId("hardware-routing");
}

const routing = (): ExternalRouting => {
  const k = store().project!.devices[id]!.kind;
  if (k.type === "Builtin" && (k.device.type === "ExternalInstrument" || k.device.type === "ExternalAudioEffect")) return k.device.routing;
  throw new Error("not external");
};

async function pick(panel: HTMLElement, name: string, option: string) {
  await act(async () => {
    fireEvent.click(within(panel).getByRole("combobox", { name }));
  });
  await act(async () => {
    fireEvent.click(screen.getByRole("option", { name: option }));
  });
  await flush();
}

describe("HardwareRoutingWidget", () => {
  it("routes an instrument to a MIDI port and channel and back from an input (undoable)", async () => {
    const panel = await render("ExternalInstrument");
    await pick(panel, "MIDI output", "Mock Synth");
    await pick(panel, "MIDI channel", "Ch 10");
    await pick(panel, "Audio return", "In 3/4");
    expect(routing()).toEqual({ midi_out: "Mock Synth", midi_channel: 10, audio_send: null, audio_return: { first: 2, count: 2 } });
    // The mock has no hardware clock: measuring fails with a readable message.
    const measure = within(panel).getByRole("button", { name: /Measure/ });
    expect(measure).not.toBeDisabled();
    await act(async () => {
      fireEvent.click(measure);
    });
    await flush();
    expect(within(panel).getByRole("status").textContent).toMatch(/desktop app/);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    await flush();
    expect(routing().audio_return).toBeNull();
  });

  it("sends an effect to outputs and marks ports this machine lacks", async () => {
    const panel = await render("ExternalAudioEffect");
    expect(within(panel).queryByRole("combobox", { name: "MIDI output" })).toBeNull();
    expect(within(panel).getByRole("button", { name: /Measure/ })).toBeDisabled();
    await pick(panel, "Audio send", "Out 3/4");
    expect(routing().audio_send).toEqual({ first: 2, count: 2 });
    // A project from another machine: return on inputs 9/10.
    await act(async () => {
      await mock!.send(cmd("External", { type: "SetRouting", device: id, routing: { ...routing(), audio_return: { first: 8, count: 2 } } }));
    });
    await flush();
    expect(within(panel).getByRole("note").textContent).toMatch(/audio return on this machine/);
    expect(within(panel).getByRole("combobox", { name: "Audio return" }).textContent).toMatch(/In 9\/10 \(not connected\)/);
  });
});
