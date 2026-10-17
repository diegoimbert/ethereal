import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import { playheadStore, useProjectStore } from "@/state";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import { mockNotice, webNotice } from "./host";
import { MidiLearnPanel, MidiLearnRoot } from "./index";
import { useMidiLearnStore } from "./store";
import { midiTarget } from "./targets";
import { MIDI_MODE_CLASS } from "./useMidiMode";
import { pickOption } from "@/kit/testing";

const store = () => useProjectStore.getState();

class MidiMock extends MockTransport {
  sent: Command[] = [];
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
  midiCommands(type: string): Command[] {
    return this.sent.filter((c) => c.domain === "MidiMap" && c.command.type === type);
  }
}

async function setup(kind?: "wasm") {
  const mock = new MidiMock({ timers: "manual", seed: 5 });
  if (kind) Object.defineProperty(mock, "kind", { value: kind });
  const utils = render(<Shell mock={mock} panel />);
  await waitFor(() => {
    if (!store().project) throw new Error("not connected");
  });
  return { mock, ...utils };
}

/** The app shell's part: `MidiLearnRoot` always mounted, the MIDI tab only when `panel`. */
function Shell({ mock, panel }: { mock: MockTransport; panel: boolean }) {
  return (
    <TransportProvider transport={mock}>
      <MidiLearnRoot />
      {panel && <MidiLearnPanel />}
      <FakeStrip />
    </TransportProvider>
  );
}

/** A track-control-like control (real controls carry `data-midi-target` the same way). */
function FakeStrip() {
  const track = useProjectStore((s) => (s.project ? Object.keys(s.project.tracks)[0] : undefined));
  if (!track) return null;
  return (
    <div className="eth-strip" data-track={track}>
      <div className="eth-knob eth-strip__pan" data-testid="pan" {...midiTarget({ type: "Param", target: { type: "TrackPan", track } })}>
        <span data-testid="pan-inner" />
      </div>
      <button type="button" className="eth-strip__name">
        name
      </button>
    </div>
  );
}

afterEach(() => {
  store().reset();
  playheadStore.reset();
  useMidiLearnStore.getState().reset();
});

describe("MidiLearnPanel", () => {
  it("learns a clicked control from the next MIDI message, then edits and removes the mapping", async () => {
    const { mock } = await setup();
    const track = Object.values(store().project!.tracks)[0]!;
    expect(screen.getByText("no midi mappings.")).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(mockNotice());

    fireEvent.click(screen.getByRole("switch", { name: "midi mode" }));
    await waitFor(() => expect(document.body).toHaveClass(MIDI_MODE_CLASS));
    const pan = screen.getByTestId("pan");
    expect(pan).toHaveAttribute("data-midi-mappable");
    // Non-mappable controls are left alone.
    expect(screen.getByText("name")).not.toHaveAttribute("data-midi-mappable");

    // Click the control: learn starts (the click does not reach the control).
    fireEvent.pointerDown(screen.getByTestId("pan-inner"), { button: 0 });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toEqual({ type: "Param", target: { type: "TrackPan", track: track.id } }));
    expect(screen.getByRole("status")).toHaveTextContent(`${track.name} · pan`);
    await waitFor(() => expect(pan).toHaveAttribute("data-midi-learning"));

    // Move a hardware knob.
    act(() => mock.simulateMidiInput("mock-midi", [0xb0, 21, 64]));
    await waitFor(() => expect(Object.values(store().project!.midi_mappings)).toHaveLength(1));
    expect(useMidiLearnStore.getState().learning).toBeNull();
    const list = screen.getByRole("list", { name: "midi mappings" });
    expect(within(list).getByText(`${track.name} · pan`)).toBeInTheDocument();
    expect(within(list).getByText("cc 21 · ch 1 · mock-midi")).toBeInTheDocument();
    await waitFor(() => expect(pan).toHaveAttribute("data-midi-mapped"));

    // Edit the mode and the range.
    pickOption(screen.getByRole("combobox", { name: `mode of ${track.name} · pan` }), { value: "Relative:BinaryOffset" });
    await waitFor(() => expect(Object.values(store().project!.midi_mappings)[0]!.mode).toEqual({ type: "Relative", encoding: "BinaryOffset" }));
    const max = screen.getByLabelText(`maximum of ${track.name} · pan`);
    fireEvent.change(max, { target: { value: "40" } });
    fireEvent.keyDown(max, { key: "Enter" });
    await waitFor(() => expect(Object.values(store().project!.midi_mappings)[0]!.max).toBeCloseTo(0.4));

    // Remove it.
    fireEvent.click(screen.getByRole("button", { name: `remove mapping ${track.name} · pan` }));
    await waitFor(() => expect(Object.values(store().project!.midi_mappings)).toHaveLength(0));
    expect(mock.midiCommands("Unmap")).toHaveLength(1);

    // Turning MIDI mode off clears the marks.
    fireEvent.click(screen.getByRole("switch", { name: "midi mode" }));
    await waitFor(() => expect(document.body).not.toHaveClass(MIDI_MODE_CLASS));
    expect(pan).not.toHaveAttribute("data-midi-mappable");
  });

  it("cancels a learn by clicking the control again, with Escape or the Cancel button", async () => {
    const { mock } = await setup();
    fireEvent.click(screen.getByRole("switch", { name: "midi mode" }));
    const pan = await screen.findByTestId("pan");
    fireEvent.pointerDown(pan, { button: 0 });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).not.toBeNull());
    fireEvent.pointerDown(pan, { button: 0 });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toBeNull());

    fireEvent.pointerDown(pan, { button: 0 });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).not.toBeNull());
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toBeNull());

    fireEvent.pointerDown(pan, { button: 0 });
    fireEvent.click(await screen.findByRole("button", { name: "cancel" }));
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toBeNull());
    expect(mock.midiCommands("Learn").map((c) => (c.command as { target: unknown }).target === null)).toEqual([false, true, false, true, false, true]);
  });

  it("learns transport actions from the target menu and shows input activity", async () => {
    const { mock } = await setup();
    fireEvent.click(screen.getByRole("switch", { name: "midi mode" }));
    pickOption(await screen.findByRole("combobox", { name: "learn other target" }), { value: "transport:ToggleLoop" });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toEqual({ type: "Transport", action: "ToggleLoop" }));
    act(() => mock.simulateMidiInput("mock-midi", [0x90, 60, 100]));
    const list = await screen.findByRole("list", { name: "midi mappings" });
    expect(within(list).getByText("transport · loop")).toBeInTheDocument();
    expect(screen.getByText("last input: note C3 · ch 1 · mock-midi")).toBeInTheDocument();
  });

  it("keeps MIDI mode and learns with the MIDI tab closed", async () => {
    const { mock, rerender } = await setup();
    const track = Object.values(store().project!.tracks)[0]!;
    fireEvent.click(screen.getByRole("switch", { name: "midi mode" }));
    await waitFor(() => expect(document.body).toHaveClass(MIDI_MODE_CLASS));

    // Close the MIDI tab: MIDI mode stays on and clicking a control still learns it.
    rerender(<Shell mock={mock} panel={false} />);
    expect(screen.queryByRole("switch", { name: "midi mode" })).toBeNull();
    expect(document.body).toHaveClass(MIDI_MODE_CLASS);
    const pan = screen.getByTestId("pan");
    await waitFor(() => expect(pan).toHaveAttribute("data-midi-mappable"));
    fireEvent.pointerDown(pan, { button: 0 });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toEqual({ type: "Param", target: { type: "TrackPan", track: track.id } }));
    await waitFor(() => expect(pan).toHaveAttribute("data-midi-learning"));

    // The engine's learn events still reach the store: the mapping is created and learning ends.
    act(() => mock.simulateMidiInput("mock-midi", [0xb0, 21, 64]));
    await waitFor(() => expect(Object.values(store().project!.midi_mappings)).toHaveLength(1));
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toBeNull());
    expect(useMidiLearnStore.getState().learned).toBe(Object.keys(store().project!.midi_mappings)[0]);
    await waitFor(() => expect(pan).toHaveAttribute("data-midi-mapped"));

    // Reopening the tab shows the same state and the mapping.
    rerender(<Shell mock={mock} panel />);
    expect(screen.getByRole("switch", { name: "midi mode" })).toBeChecked();
    expect(within(screen.getByRole("list", { name: "midi mappings" })).getByText(`${track.name} · pan`)).toBeInTheDocument();
  });

  it("explains that MIDI learn needs the desktop app on the web build", async () => {
    await setup("wasm");
    expect(screen.getByRole("note")).toHaveTextContent(webNotice());
    expect(screen.getByRole("switch", { name: "midi mode" })).toBeDisabled();
  });
});
