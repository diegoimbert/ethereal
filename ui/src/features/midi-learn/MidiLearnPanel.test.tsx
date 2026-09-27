import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import { playheadStore, useProjectStore } from "@/state";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import { WEB_NOTICE } from "./host";
import { MidiLearnPanel } from "./index";
import { useMidiLearnStore } from "./store";
import { MIDI_MODE_CLASS } from "./useMidiMode";

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
  const utils = render(
    <TransportProvider transport={mock}>
      <MidiLearnPanel />
      <FakeStrip />
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!store().project) throw new Error("not connected");
  });
  return { mock, ...utils };
}

/** A mixer-strip-like control (the real strip lives in the mixer feature). */
function FakeStrip() {
  const track = useProjectStore((s) => (s.project ? Object.keys(s.project.tracks)[0] : undefined));
  if (!track) return null;
  return (
    <div className="eth-strip" data-track={track}>
      <div className="eth-knob eth-strip__pan" data-testid="pan">
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
    expect(screen.getByText("No MIDI mappings.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("switch", { name: "MIDI mode" }));
    await waitFor(() => expect(document.body).toHaveClass(MIDI_MODE_CLASS));
    const pan = screen.getByTestId("pan");
    expect(pan).toHaveAttribute("data-midi-mappable");
    // Non-mappable controls are left alone.
    expect(screen.getByText("name")).not.toHaveAttribute("data-midi-mappable");

    // Click the control: learn starts (the click does not reach the control).
    fireEvent.pointerDown(screen.getByTestId("pan-inner"), { button: 0 });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toEqual({ type: "Param", target: { type: "TrackPan", track: track.id } }));
    expect(screen.getByRole("status")).toHaveTextContent(`${track.name} · Pan`);
    await waitFor(() => expect(pan).toHaveAttribute("data-midi-learning"));

    // Move a hardware knob.
    act(() => mock.simulateMidiInput("mock-midi", [0xb0, 21, 64]));
    await waitFor(() => expect(Object.values(store().project!.midi_mappings)).toHaveLength(1));
    expect(useMidiLearnStore.getState().learning).toBeNull();
    const list = screen.getByRole("list", { name: "MIDI mappings" });
    expect(within(list).getByText(`${track.name} · Pan`)).toBeInTheDocument();
    expect(within(list).getByText("CC 21 · Ch 1 · mock-midi")).toBeInTheDocument();
    await waitFor(() => expect(pan).toHaveAttribute("data-midi-mapped"));

    // Edit the mode and the range.
    fireEvent.change(screen.getByLabelText(`Mode of ${track.name} · Pan`), { target: { value: "Relative:BinaryOffset" } });
    await waitFor(() =>
      expect(Object.values(store().project!.midi_mappings)[0]!.mode).toEqual({ type: "Relative", encoding: "BinaryOffset" }),
    );
    const max = screen.getByLabelText(`Maximum of ${track.name} · Pan`);
    fireEvent.change(max, { target: { value: "40" } });
    fireEvent.keyDown(max, { key: "Enter" });
    await waitFor(() => expect(Object.values(store().project!.midi_mappings)[0]!.max).toBeCloseTo(0.4));

    // Remove it.
    fireEvent.click(screen.getByRole("button", { name: `Remove mapping ${track.name} · Pan` }));
    await waitFor(() => expect(Object.values(store().project!.midi_mappings)).toHaveLength(0));
    expect(mock.midiCommands("Unmap")).toHaveLength(1);

    // Turning MIDI mode off clears the marks.
    fireEvent.click(screen.getByRole("switch", { name: "MIDI mode" }));
    await waitFor(() => expect(document.body).not.toHaveClass(MIDI_MODE_CLASS));
    expect(pan).not.toHaveAttribute("data-midi-mappable");
  });

  it("cancels a learn by clicking the control again, with Escape or the Cancel button", async () => {
    const { mock } = await setup();
    fireEvent.click(screen.getByRole("switch", { name: "MIDI mode" }));
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
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toBeNull());
    expect(mock.midiCommands("Learn").map((c) => (c.command as { target: unknown }).target === null)).toEqual([
      false,
      true,
      false,
      true,
      false,
      true,
    ]);
  });

  it("learns transport actions from the target menu and shows input activity", async () => {
    const { mock } = await setup();
    fireEvent.click(screen.getByRole("switch", { name: "MIDI mode" }));
    fireEvent.change(await screen.findByLabelText("Learn other target"), { target: { value: "transport:ToggleLoop" } });
    await waitFor(() => expect(useMidiLearnStore.getState().learning).toEqual({ type: "Transport", action: "ToggleLoop" }));
    act(() => mock.simulateMidiInput("mock-midi", [0x90, 60, 100]));
    const list = await screen.findByRole("list", { name: "MIDI mappings" });
    expect(within(list).getByText("Transport · Loop")).toBeInTheDocument();
    expect(screen.getByText("Last input: Note C3 · Ch 1 · mock-midi")).toBeInTheDocument();
  });

  it("explains that MIDI learn needs the desktop app on the web build", async () => {
    await setup("wasm");
    expect(screen.getByRole("note")).toHaveTextContent(WEB_NOTICE);
    expect(screen.getByRole("switch", { name: "MIDI mode" })).toBeDisabled();
  });
});
