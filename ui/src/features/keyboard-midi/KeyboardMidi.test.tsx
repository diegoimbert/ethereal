import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tracksOrdered, useProjectStore, useSelectionStore } from "@/state";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import type { Command, Event, ReplyValue } from "@/generated";
import { requestedCaptureCodes } from "@/features/plugin-keys";
import { cmd, type EngineTransport } from "@/transport";
import { bindTransport, handleForwardedKey, pluginCaptureCodes, setEnabled, useKeyboardMidi } from "./keyboard";
import { KeyboardMidiButton } from "./KeyboardMidiButton";

const toggle = () => screen.getByRole("button", { name: "computer MIDI keyboard" });
const status = () => screen.getByTestId("keyboard-midi-status");
const down = (code: string, init: Partial<KeyboardEventInit> = {}) => fireEvent.keyDown(window, { code, key: code.replace(/^Key/, "").toLowerCase(), ...init });
const up = (code: string, init: Partial<KeyboardEventInit> = {}) => fireEvent.keyUp(window, { code, key: code.replace(/^Key/, "").toLowerCase(), ...init });

beforeEach(() => {
  setEnabled(false);
  useKeyboardMidi.setState({ octave: 3, velocity: 100, held: {} });
});

afterEach(() => {
  setEnabled(false);
  resetStores();
  useSelectionStore.getState().selectTrack(null);
});

/** Render the toggle on a mock engine and turn the keyboard on. */
async function on() {
  const r = await renderWithMock(<KeyboardMidiButton />);
  fireEvent.click(toggle());
  expect(toggle()).toHaveAttribute("aria-pressed", "true");
  return r;
}

describe("KeyboardMidiButton", () => {
  it("toggles with the button and with M, and shows octave and velocity while on", async () => {
    render(<KeyboardMidiButton />);
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByTestId("keyboard-midi-status")).toBeNull();
    fireEvent.click(toggle());
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    expect(status()).toHaveTextContent("C3");
    expect(status()).toHaveTextContent("vel100");
    expect(status()).toHaveAccessibleName("octave C3, velocity 100");
    down("KeyM", { key: "m" });
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
    down("KeyM", { key: "m" });
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    // Not while typing.
    const input = document.createElement("input");
    document.body.append(input);
    fireEvent.keyDown(input, { code: "KeyM", key: "m" });
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    input.remove();
  });

  it("Z/X change the octave and C/V the velocity (repeat ignored)", async () => {
    render(<KeyboardMidiButton />);
    fireEvent.click(toggle());
    down("KeyX");
    expect(status()).toHaveTextContent("C4");
    down("KeyX", { repeat: true });
    expect(status()).toHaveTextContent("C4");
    down("KeyZ");
    down("KeyZ");
    expect(status()).toHaveTextContent("C2");
    down("KeyV");
    expect(status()).toHaveTextContent("vel120");
    down("KeyC");
    down("KeyC");
    expect(status()).toHaveTextContent("vel80");
  });
});

describe("computer MIDI keyboard notes", () => {
  it("plays notes on key down / up as live MIDI, ignoring auto-repeat", async () => {
    const { mock } = await on();
    down("KeyA");
    down("KeyA", { repeat: true });
    down("KeyW");
    await waitFor(() =>
      expect(mock.liveMidi).toEqual([
        [0x90, 60, 100],
        [0x90, 61, 100],
      ]),
    );
    expect(status()).toHaveAttribute("data-playing", "true");
    up("KeyA");
    up("KeyW");
    await waitFor(() =>
      expect(mock.liveMidi.slice(2)).toEqual([
        [0x80, 60, 0],
        [0x80, 61, 0],
      ]),
    );
    expect(status()).not.toHaveAttribute("data-playing");
    down("Semicolon", { key: ";" });
    await waitFor(() => expect(mock.liveMidi.at(-1)).toEqual([0x90, 76, 100]));
    up("Semicolon", { key: ";" });
  });

  it("keeps note keys from other shortcuts, but not Space or modified keys", async () => {
    await on();
    const other = vi.fn();
    window.addEventListener("keydown", other);
    try {
      down("KeyA");
      down("KeyZ");
      expect(other).not.toHaveBeenCalled();
      fireEvent.keyDown(window, { code: "Space", key: " " });
      down("KeyA", { ctrlKey: true });
      down("KeyB");
      expect(other).toHaveBeenCalledTimes(3);
    } finally {
      window.removeEventListener("keydown", other);
      up("KeyA");
    }
  });

  it("ends the note a key started even after an octave or velocity change", async () => {
    const { mock } = await on();
    down("KeyA");
    down("KeyX");
    down("KeyV");
    down("KeyS");
    up("KeyA");
    up("KeyS");
    await waitFor(() =>
      expect(mock.liveMidi).toEqual([
        [0x90, 60, 100],
        [0x90, 74, 120],
        [0x80, 60, 0],
        [0x80, 74, 0],
      ]),
    );
  });

  it("releases held notes on blur, on hide and when turned off (no stuck notes)", async () => {
    const { mock } = await on();
    down("KeyA");
    down("KeyD");
    fireEvent.blur(window);
    await waitFor(() => expect(mock.liveMidi.filter((m) => m[0] === 0x80).map((m) => m[1])).toEqual([60, 64]));
    // The key up after the blur sends nothing more.
    up("KeyA");
    await act(() => Promise.resolve());
    expect(mock.liveMidi).toHaveLength(4);

    down("KeyF");
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    fireEvent(document, new Event("visibilitychange"));
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    await waitFor(() => expect(mock.liveMidi.at(-1)).toEqual([0x80, 65, 0]));

    down("KeyG");
    fireEvent.click(toggle());
    await waitFor(() => expect(mock.liveMidi.at(-1)).toEqual([0x80, 67, 0]));
    expect(useKeyboardMidi.getState().held).toEqual({});
  });

  it("does nothing while off, or while typing in a text field", async () => {
    const { mock } = await renderWithMock(<KeyboardMidiButton />);
    const other = vi.fn();
    window.addEventListener("keydown", other);
    down("KeyA");
    up("KeyA");
    expect(other).toHaveBeenCalledTimes(1);
    window.removeEventListener("keydown", other);
    fireEvent.click(toggle());
    const input = document.createElement("input");
    document.body.append(input);
    fireEvent.keyDown(input, { code: "KeyA", key: "a" });
    fireEvent.keyUp(input, { code: "KeyA", key: "a" });
    input.remove();
    await act(() => Promise.resolve());
    expect(mock.liveMidi).toEqual([]);
  });

  it("arms the selected MIDI track when no MIDI track would hear the notes", async () => {
    const { mock } = await renderWithMock(<KeyboardMidiButton />);
    const project = useProjectStore.getState().project!;
    const midi = tracksOrdered(project)
      .filter((t) => t.kind === "Midi")
      .at(-1)!;
    useSelectionStore.getState().selectTrack(midi.id);
    expect(useProjectStore.getState().armedTracks).toEqual([]);
    fireEvent.click(toggle());
    await waitFor(() => expect(useProjectStore.getState().armedTracks).toEqual([midi.id]));
    expect(toggle()).toHaveAttribute("title", expect.stringContaining(`plays on “${midi.name}”`));
    // Already heard: nothing more is armed.
    await mock.send(cmd("Recording", { type: "Arm", track: midi.id, armed: false, exclusive: false }));
    const other = tracksOrdered(project).find((t) => t.kind === "Midi" && t.id !== midi.id)!;
    await mock.send(cmd("Recording", { type: "Arm", track: other.id, armed: true, exclusive: false }));
    await waitFor(() => expect(useProjectStore.getState().armedTracks).toEqual([other.id]));
    down("KeyA");
    up("KeyA");
    await waitFor(() => expect(mock.liveMidi).toHaveLength(2));
    expect(useProjectStore.getState().armedTracks).toEqual([other.id]);
  });

  it("plays keys forwarded from a plugin window while on, and leaves them to the plugin while off", async () => {
    const { mock } = await on();
    const key = (code: string, isDown: boolean) =>
      handleForwardedKey({ code, key: code.slice(3).toLowerCase(), down: isDown, shiftKey: false, ctrlKey: false, altKey: false, metaKey: false });
    expect(key("KeyH", true)).toBe(true);
    expect(key("KeyH", false)).toBe(true);
    await waitFor(() =>
      expect(mock.liveMidi).toEqual([
        [0x90, 69, 100],
        [0x80, 69, 0],
      ]),
    );
    expect(key("KeyB", true)).toBe(false);
    fireEvent.click(toggle());
    expect(key("KeyH", true)).toBe(false);
  });

  it("registers its keys with plugin editor windows only while on, and plays the keys they forward", async () => {
    let listener: ((e: Event) => void) | null = null;
    const sent: Command[] = [];
    const fake = {
      send: (c: Command) => {
        sent.push(c);
        return Promise.resolve({ type: "Unit" } as ReplyValue);
      },
      onEvent: (l: (e: Event) => void) => {
        listener = l;
        return () => (listener = null);
      },
    } as unknown as EngineTransport;
    const unbind = bindTransport(fake);
    try {
      // Off: plugins keep their own typing.
      expect(requestedCaptureCodes(fake)).toEqual([]);
      setEnabled(true);
      const codes = requestedCaptureCodes(fake);
      expect(codes).toEqual(pluginCaptureCodes(true));
      for (const c of ["KeyA", "Semicolon", "KeyZ", "KeyV", "KeyM"]) expect(codes).toContain(c);
      expect(sent).toContainEqual(cmd("Plugin", { type: "SetEditorKeyCapture", codes }));

      const key = (code: string, down: boolean): Event =>
        ({
          type: "Plugin",
          event: {
            type: "EditorKey",
            device: "dev",
            key: { key: code.slice(3).toLowerCase(), code, shift: false, ctrl: false, alt: false, meta: false, down, repeat: false },
          },
        }) as unknown as Event;
      listener!(key("KeyJ", true));
      listener!(key("KeyJ", false));
      const live = sent.filter((c) => c.domain === "Recording").map((c) => (c.command as { data: number[] }).data);
      expect(live).toEqual([
        [0x90, 71, 100],
        [0x80, 71, 0],
      ]);
      // M from inside the plugin turns it off; the capture set goes back to empty.
      listener!(key("KeyM", true));
      expect(useKeyboardMidi.getState().enabled).toBe(false);
      expect(requestedCaptureCodes(fake)).toEqual([]);
    } finally {
      unbind();
    }
  });

  it("feeds MIDI capture like a hardware input", async () => {
    const { mock } = await on();
    for (const code of ["KeyA", "KeyS", "KeyD"]) {
      down(code);
      up(code);
    }
    await waitFor(() => expect(mock.liveMidi).toHaveLength(6));
    const reply = await mock.send(cmd("Capture", { type: "Status" }));
    expect(reply).toMatchObject({ type: "CaptureStatus", status: { notes: 3 } });
  });
});
