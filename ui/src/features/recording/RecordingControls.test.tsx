import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, Event, ReplyValue } from "@/generated";
import { playheadStore, tracksOrdered, useProjectStore } from "@/state";
import { cmd, CommandFailedError, MockTransport, TransportProvider, type SendOptions, type Unsubscribe } from "@/transport";
import { pickOption } from "@/kit/testing";
import { RecordingControls, UNSUPPORTED_TOOLTIP } from "./index";

const store = () => useProjectStore.getState();

/** MockTransport plus the host-side recording commands the mock doesn't implement. */
class RecordingMock extends MockTransport {
  sent: Command[] = [];
  inputs: "supported" | "unsupported" = "supported";
  private listeners: ((e: Event) => void)[] = [];

  override onEvent(listener: (event: Event) => void): Unsubscribe {
    this.listeners.push(listener);
    const off = super.onEvent(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
      off();
    };
  }

  fire(event: Event): void {
    for (const l of this.listeners) l(event);
  }

  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    if (command.domain === "Recording") {
      const c = command.command;
      if (c.type === "ListInputs" && this.inputs === "unsupported") {
        throw new CommandFailedError({ code: "Unsupported", message: "input listing is not available on this host" }, command);
      }
      if (c.type === "SetPunch") {
        this.fire({ type: "Recording", event: { type: "PunchChanged", enabled: c.enabled } });
        return { type: "Unit" };
      }
      if (c.type === "SetRecording") {
        const t = store().transport!;
        this.fire({ type: "Transport", state: { ...t, recording: c.enabled, playing: t.playing || c.enabled } });
        return { type: "Unit" };
      }
    }
    return super.send(command, opts);
  }
}

async function renderControls(inputs: "supported" | "unsupported" = "supported") {
  const mock = new RecordingMock({ timers: "manual", seed: 3 });
  mock.inputs = inputs;
  render(
    <TransportProvider transport={mock}>
      <RecordingControls />
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!store().project) throw new Error("not connected");
  });
  await waitFor(() => expect(mock.sent.some((c) => c.domain === "Recording" && c.command.type === "ListInputs")).toBe(true));
  return mock;
}

afterEach(() => {
  store().reset();
  playheadStore.reset();
});

describe("RecordingControls", () => {
  it("renders disabled without an engine", () => {
    render(<RecordingControls />);
    expect(screen.getByRole("button", { name: "Record armed tracks" })).toBeDisabled();
    expect(screen.getByLabelText("Count-in")).toBeDisabled();
  });

  it("records and shows the recording indicator", async () => {
    const mock = await renderControls();
    const record = screen.getByRole("button", { name: "Record armed tracks" });
    await waitFor(() => expect(record).toBeEnabled());
    expect(screen.getByTestId("recording-indicator")).toHaveTextContent("");
    fireEvent.click(record);
    await waitFor(() => expect(screen.getByTestId("recording-indicator")).toHaveTextContent("REC"));
    expect(record).toHaveAttribute("aria-pressed", "true");
    expect(mock.sent).toContainEqual(cmd("Recording", { type: "SetRecording", enabled: true }));
    fireEvent.click(record);
    await waitFor(() => expect(screen.getByTestId("recording-indicator")).toHaveTextContent(""));
    expect(mock.sent).toContainEqual(cmd("Recording", { type: "SetRecording", enabled: false }));
  });

  it("disables recording with a tooltip when the host has no inputs (web)", async () => {
    await renderControls("unsupported");
    const record = screen.getByRole("button", { name: "Record armed tracks" });
    await waitFor(() => expect(record).toBeDisabled());
    expect(record).toHaveAttribute("title", UNSUPPORTED_TOOLTIP);
    expect(record.parentElement).toHaveAttribute("title", UNSUPPORTED_TOOLTIP);
    expect(screen.getByRole("button", { name: "Punch in/out" })).toBeDisabled();
  });

  it("sets the count-in (an undoable project setting)", async () => {
    await renderControls();
    const countIn = screen.getByRole("combobox", { name: "Count-in" });
    expect(countIn).toHaveTextContent("Off");
    pickOption(countIn, { value: "2" });
    await waitFor(() => expect(store().project!.settings.count_in_bars).toBe(2));
    await waitFor(() => expect(countIn).toHaveTextContent("2 bars"));
  });

  it("toggles punch from the engine's PunchChanged event", async () => {
    await renderControls();
    const punch = screen.getByRole("button", { name: "Punch in/out" });
    expect(punch).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(punch);
    await waitFor(() => expect(punch).toHaveAttribute("aria-pressed", "true"));
  });

  it("arms tracks and sets their input and monitoring", async () => {
    const mock = await renderControls();
    fireEvent.click(screen.getByRole("button", { name: "Inputs" }));
    const panel = await screen.findByRole("dialog", { name: "Recording inputs" });
    const audio = tracksOrdered(store().project!).find((t) => t.kind === "Audio")!;
    const row = within(panel).getByText(audio.name).closest("tr")!;

    fireEvent.click(within(row).getByRole("button", { name: `Arm ${audio.name}` }));
    await waitFor(() => expect(store().armedTracks).toContain(audio.id));

    // Mono input 2 ("Mock In 2" from the mock's input list).
    const input = within(row).getByRole("combobox", { name: `Input of ${audio.name}` });
    fireEvent.click(input);
    // The list fills in once the mock's input list arrives.
    fireEvent.click(await screen.findByRole("option", { name: "Mock In 2" }));
    await waitFor(() => expect(store().project!.tracks[audio.id]!.input).toEqual({ type: "Audio", first: 1, count: 1 }));

    const monitor = within(row).getByRole("combobox", { name: `Monitoring of ${audio.name}` });
    pickOption(monitor, { value: "Off" });
    await waitFor(() => expect(store().project!.tracks[audio.id]!.monitor).toBe("Off"));

    // The input device list comes from the host-handled engine config.
    const device = within(panel).getByRole("combobox", { name: "Audio input device" });
    await waitFor(() => expect(device).toHaveTextContent("Mock Input"));
    await act(() => Promise.resolve());
    expect(mock.sent.some((c) => c.domain === "Engine" && c.command.type === "ListAudioDevices")).toBe(true);
  });
});
