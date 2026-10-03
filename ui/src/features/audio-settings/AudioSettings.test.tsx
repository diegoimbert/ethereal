import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AudioConfig, Command } from "@/generated";
import { pickOption } from "@/kit/testing";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useCollabStore } from "@/features/collab/store";
import { useShareSettings } from "@/features/share";
import { AudioSettingsDialog, openAudioSettings, openSettings, promptForInputIfNone, useAudioSettings } from "./index";
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
    expect(screen.queryByText(/Loading devices/)).toBeNull();
    const input = screen.getByRole("combobox", { name: "Input device" });
    expect(input).toHaveTextContent("None (no recording)");
    expect(screen.getByRole("combobox", { name: "Output device" })).toHaveTextContent("Mock Output (system default)");
    expect(await screen.findByTestId("audio-status")).toHaveTextContent("48000 Hz");

    pickOption(input, /Mock Input/);
    await waitFor(() => expect(applied).toHaveLength(1));
    expect(applied[0]).toMatchObject({ input_device: "Mock Input", output_device: null, sample_rate: null });
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Input device" })).toHaveTextContent("Mock Input"));

    pickOption(screen.getByRole("combobox", { name: "Buffer size" }), { value: "128" });
    await waitFor(() => expect(applied[1]).toMatchObject({ buffer_size: 128, input_device: null }));

    // Refresh re-reads the devices.
    const sends = vi.mocked(mock.send).mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Refresh devices" }));
    await waitFor(() => expect(vi.mocked(mock.send).mock.calls.length).toBeGreaterThan(sends));
  });

  it("arming with no input device opens the settings with a hint", async () => {
    const { mock } = await setup();
    await act(async () => promptForInputIfNone(mock));
    expect(useAudioSettings.getState()).toMatchObject({ open: true, reason: "input" });
    expect(await screen.findByText(/Choose an input device to record from/)).toBeInTheDocument();
  });

  it("has Audio | Sharing | Advanced tabs (base-115)", async () => {
    const { mock } = await setup();
    const sent: Command[] = [];
    // Record on top of setup()'s stub.
    const stub = vi.mocked(mock.send).getMockImplementation()!;
    vi.mocked(mock.send).mockImplementation(async (c, o) => {
      sent.push(c);
      return stub(c, o);
    });
    act(() => openSettings("sharing"));
    const dialog = screen.getByRole("dialog", { name: "Settings" });
    expect(screen.getByRole("tab", { name: "Sharing" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByTestId("settings-sharing")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "Ada" } });
    fireEvent.blur(screen.getByLabelText("Your name"));
    await waitFor(() => expect(sent).toContainEqual({ domain: "Share", command: { type: "SetIdentity", name: "Ada", color: null } }));
    fireEvent.click(screen.getByRole("switch", { name: /Resume sharing/ }));
    expect(useShareSettings.getState().resumeOnOpen).toBe(false);

    fireEvent.click(screen.getByRole("tab", { name: "Advanced" }));
    const signal = screen.getByLabelText("Signaling server");
    fireEvent.change(signal, { target: { value: "ftp://nope" } });
    expect(screen.getByRole("alert")).toHaveTextContent(/http\(s\) address/);
    fireEvent.change(signal, { target: { value: "https://signal.example.com" } });
    fireEvent.blur(signal);
    await waitFor(() =>
      expect(sent).toContainEqual({ domain: "Share", command: { type: "SetServers", signal_url: "https://signal.example.com", invite_origin: null } }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Add a STUN or TURN server" }));
    fireEvent.change(screen.getByLabelText("ICE server 1"), { target: { value: "turn:turn.example.com:3478" } });
    fireEvent.change(screen.getByLabelText("ICE server 1 username"), { target: { value: "u" } });
    fireEvent.blur(screen.getByLabelText("ICE server 1 username"));
    await waitFor(() =>
      expect(sent).toContainEqual({
        domain: "Collab",
        command: { type: "SetIceServers", servers: [{ urls: ["turn:turn.example.com:3478"], username: "u", credential: null }] },
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Remove ICE server 1" }));
    await waitFor(() => expect(sent.at(-1)).toEqual({ domain: "Collab", command: { type: "SetIceServers", servers: null } }));

    // The relay join form opens from here (the settings close).
    fireEvent.click(screen.getByRole("button", { name: "Join a relay session…" }));
    expect(useCollabStore.getState().dialogOpen).toBe(true);
    expect(useAudioSettings.getState().open).toBe(false);
    expect(dialog).toBeInTheDocument();
    act(() => useCollabStore.getState().setDialogOpen(false));
    // The gear reopens the last tab.
    act(() => openSettings());
    expect(screen.getByRole("tab", { name: "Advanced" })).toHaveAttribute("aria-selected", "true");
    act(() => openAudioSettings());
    expect(screen.getByRole("tab", { name: "Audio" })).toHaveAttribute("aria-selected", "true");
    localStorage.clear();
    useShareSettings.getState().reload();
  });
});
