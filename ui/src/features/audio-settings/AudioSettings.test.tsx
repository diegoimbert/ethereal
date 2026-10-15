import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AudioConfig, Command } from "@/generated";
import { pickOption } from "@/kit/testing";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { AudioSettingsDialog, openAudioSettings, promptForInputIfNone, useAudioSettings } from "./index";
import { loadAudioDevices } from "./store";

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
  // The dialog loaded the devices when it mounted; reload them through the stub.
  await act(async () => loadAudioDevices(mock));
  return { mock, applied };
}

describe("AudioSettingsDialog", () => {
  it("lists the devices and applies each change on its own", async () => {
    const { mock, applied } = await setup();
    act(() => openAudioSettings());
    // Already loaded: no "Loading devices…" step when the dialog opens.
    expect(screen.queryByText(/loading devices/)).toBeNull();
    const input = screen.getByRole("combobox", { name: "input device" });
    expect(input).toHaveTextContent("none (no recording)");
    expect(screen.getByRole("combobox", { name: "output device" })).toHaveTextContent("Mock Output (system default)");
    expect(await screen.findByTestId("audio-status")).toHaveTextContent("48000 Hz");

    pickOption(input, /Mock Input/);
    await waitFor(() => expect(applied).toHaveLength(1));
    expect(applied[0]).toMatchObject({ input_device: "Mock Input", output_device: null, sample_rate: null });
    await waitFor(() => expect(screen.getByRole("combobox", { name: "input device" })).toHaveTextContent("Mock Input"));

    pickOption(screen.getByRole("combobox", { name: "buffer size" }), { value: "128" });
    await waitFor(() => expect(applied[1]).toMatchObject({ buffer_size: 128, input_device: null }));
    // The select shows what the device runs (the mock engine stays at 256), and says so.
    expect(await screen.findByTestId("buffer-note")).toHaveTextContent("the device runs 256 samples (128 asked; it supports 32–2048)");
    expect(screen.getByRole("combobox", { name: "buffer size" })).toHaveTextContent("256 samples");

    // Refresh re-reads the devices.
    const sends = vi.mocked(mock.send).mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "refresh devices" }));
    await waitFor(() => expect(vi.mocked(mock.send).mock.calls.length).toBeGreaterThan(sends));
  });

  it("says the device is closed while no local engine plays, and follows the host's status", async () => {
    const { mock } = await setup();
    act(() => openAudioSettings());
    const line = await screen.findByTestId("audio-status");
    expect(line).toHaveTextContent("running");
    const status = useAudioSettings.getState().status!;
    const emit = (device_released: boolean) =>
      act(() =>
        (mock as unknown as { emit(e: unknown): void }).emit({
          type: "Engine",
          event: { type: "Status", status: { ...status, backend: device_released ? "null" : "mock", device_released } },
        }),
      );
    emit(true);
    expect(screen.getByTestId("audio-status")).toHaveTextContent("device closed: no local engine plays");
    expect(screen.getByTestId("audio-status")).not.toHaveTextContent("running");
    emit(false);
    expect(screen.getByTestId("audio-status")).toHaveTextContent("running · mock");
  });

  it("arming with no input device opens the settings with a hint", async () => {
    const { mock } = await setup();
    await act(async () => promptForInputIfNone(mock));
    expect(useAudioSettings.getState()).toMatchObject({ open: true, reason: "input" });
    expect(await screen.findByText(/choose an input device to record from/)).toBeInTheDocument();
  });
});
