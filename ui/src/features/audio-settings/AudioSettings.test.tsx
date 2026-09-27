import { act, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AudioConfig, Command } from "@/generated";
import { pickOption } from "@/kit/testing";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { AudioSettingsDialog, openAudioSettings, promptForInputIfNone, useAudioSettings } from "./index";

afterEach(() => {
  resetStores();
  act(() => useAudioSettings.getState().close());
  vi.restoreAllMocks();
});

/** Mock engine with a working SetAudioConfig (the mock rejects it) that updates `current`. */
async function setup() {
  const { mock } = await renderWithMock(<AudioSettingsDialog />);
  const applied: AudioConfig[] = [];
  let current: AudioConfig = { backend: null, host: null, output_device: null, input_device: null, sample_rate: null, buffer_size: null };
  const send = mock.send.bind(mock);
  vi.spyOn(mock, "send").mockImplementation(async (c: Command, o) => {
    if (c.domain === "Engine" && c.command.type === "SetAudioConfig") {
      applied.push(c.command.config);
      const patch = Object.fromEntries(Object.entries(c.command.config).filter(([, v]) => v !== null)) as Partial<AudioConfig>;
      current = { ...current, ...patch, input_device: patch.input_device === "" ? null : (patch.input_device ?? current.input_device) };
      return { type: "Unit" };
    }
    const reply = await send(c, o);
    if (reply.type === "AudioDevices") return { ...reply, devices: { ...reply.devices, current } };
    return reply;
  });
  return { mock, applied };
}

describe("AudioSettingsDialog", () => {
  it("lists the devices and applies each change on its own", async () => {
    const { applied } = await setup();
    act(() => openAudioSettings());
    const input = await screen.findByRole("combobox", { name: "Input device" });
    expect(input).toHaveTextContent("None (no recording)");
    expect(screen.getByRole("combobox", { name: "Output device" })).toHaveTextContent("Mock Output (system default)");
    expect(await screen.findByTestId("audio-status")).toHaveTextContent("48000 Hz");

    pickOption(input, /Mock Input/);
    await waitFor(() => expect(applied).toHaveLength(1));
    expect(applied[0]).toMatchObject({ input_device: "Mock Input", output_device: null, sample_rate: null });
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Input device" })).toHaveTextContent("Mock Input"));

    pickOption(screen.getByRole("combobox", { name: "Buffer size" }), { value: "128" });
    await waitFor(() => expect(applied[1]).toMatchObject({ buffer_size: 128, input_device: null }));
  });

  it("arming with no input device opens the settings with a hint", async () => {
    const { mock } = await setup();
    await act(async () => promptForInputIfNone(mock));
    expect(useAudioSettings.getState()).toMatchObject({ open: true, reason: "input" });
    expect(await screen.findByText(/Choose an input device to record from/)).toBeInTheDocument();
  });
});
