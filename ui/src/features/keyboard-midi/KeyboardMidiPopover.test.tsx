import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { setEnabled, setOctave, setVelocity, useKeyboardMidi } from "./keyboard";
import { KeyboardMidiButton } from "./KeyboardMidiButton";
import { stripStart, velocityStops } from "./notes";

const indicator = () => screen.getByTestId("keyboard-midi-status");
const dialog = () => screen.getByRole("dialog", { name: "computer keyboard octave and velocity" });
const range = () => screen.getByTestId("keyboard-midi-range");
const slider = () => screen.getByRole("slider", { name: "velocity" });
const key = (code: string) => fireEvent.keyDown(window, { code, key: code.replace(/^Key/, "").toLowerCase() });

beforeEach(() => {
  setEnabled(false);
  useKeyboardMidi.setState({ octave: 3, velocity: 100, held: {} });
});

afterEach(() => {
  setEnabled(false);
  resetStores();
});

function open() {
  render(<KeyboardMidiButton />);
  act(() => setEnabled(true));
  expect(indicator()).toHaveAttribute("title", "click to set octave and velocity");
  expect(indicator()).toHaveAttribute("aria-expanded", "false");
  fireEvent.click(indicator());
  expect(indicator()).toHaveAttribute("aria-expanded", "true");
  return dialog();
}

describe("keyboard MIDI indicator popover", () => {
  it("opens on click with the octave range and velocity, focus on the first control", () => {
    open();
    expect(range()).toHaveTextContent("c3–e4");
    expect(slider()).toHaveValue("100");
    expect(screen.getByRole("button", { name: "octave down" })).toHaveFocus();
    // The presets are the C/V stops.
    expect(velocityStops()).toEqual([1, 20, 40, 60, 80, 100, 120, 127]);
    expect(screen.getByRole("button", { name: "100" })).toHaveAttribute("aria-pressed", "true");
  });

  it("octave buttons and the keyboard strip change the octave (clamped)", () => {
    open();
    fireEvent.click(screen.getByRole("button", { name: "octave up" }));
    expect(useKeyboardMidi.getState().octave).toBe(4);
    expect(indicator()).toHaveTextContent("c4");
    expect(range()).toHaveTextContent("c4–e5");
    fireEvent.click(screen.getByRole("button", { name: "octave down" }));
    fireEvent.click(screen.getByRole("button", { name: "octave down" }));
    expect(useKeyboardMidi.getState().octave).toBe(2);

    // The strip lights the 17 played keys; clicking a key moves the octave to that key's.
    const strip = screen.getByTestId("keyboard-midi-strip");
    expect(strip.querySelectorAll("[data-played]")).toHaveLength(17);
    expect(strip.querySelector("[data-root]")).toHaveAttribute("data-note", "48");
    fireEvent.click(strip.querySelector('[data-note="67"]')!); // g3
    expect(useKeyboardMidi.getState().octave).toBe(3);
    fireEvent.click(strip.querySelector('[data-note="49"]')!); // c#2 (black key)
    expect(useKeyboardMidi.getState().octave).toBe(2);

    act(() => setOctave(8));
    expect(screen.getByRole("button", { name: "octave up" })).toBeDisabled();
    expect(range()).toHaveTextContent("c8–g8");
    act(() => setOctave(-2));
    expect(screen.getByRole("button", { name: "octave down" })).toBeDisabled();
    expect(stripStart(-2)).toBe(-2);
    expect(stripStart(8)).toBe(5);
  });

  it("the slider and presets change the velocity", () => {
    open();
    fireEvent.change(slider(), { target: { value: "64" } });
    expect(useKeyboardMidi.getState().velocity).toBe(64);
    expect(indicator()).toHaveTextContent("vel64");
    expect(screen.getByTestId("keyboard-midi-velocity")).toHaveTextContent("64");
    fireEvent.click(screen.getByRole("button", { name: "127" }));
    expect(useKeyboardMidi.getState().velocity).toBe(127);
    expect(slider()).toHaveValue("127");
    setVelocity(500);
    expect(useKeyboardMidi.getState().velocity).toBe(127);
  });

  it("shares its state with the Z/X and C/V keys while open", () => {
    open();
    act(() => void key("KeyX"));
    expect(range()).toHaveTextContent("c4–e5");
    key("KeyC");
    expect(slider()).toHaveValue("80");
    expect(screen.getByRole("button", { name: "80" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "octave down" }));
    key("KeyV");
    expect(indicator()).toHaveAccessibleName("octave c3, velocity 100");
  });

  it("note keys still play from the buttons, but not while the slider has focus", async () => {
    const { mock } = await renderWithMock(<KeyboardMidiButton />);
    act(() => setEnabled(true));
    fireEvent.click(indicator());
    const down = screen.getByRole("button", { name: "octave down" });
    fireEvent.keyDown(down, { code: "KeyA", key: "a" });
    fireEvent.keyUp(down, { code: "KeyA", key: "a" });
    await waitFor(() => expect(mock.liveMidi).toHaveLength(2));
    slider().focus();
    fireEvent.keyDown(slider(), { code: "KeyA", key: "a" });
    fireEvent.keyDown(slider(), { code: "KeyX", key: "x" });
    await act(() => Promise.resolve());
    expect(mock.liveMidi).toHaveLength(2);
    expect(useKeyboardMidi.getState().octave).toBe(3);
  });

  it("closes on Escape (focus back on the indicator) and on an outside click", () => {
    open();
    fireEvent.keyDown(slider(), { key: "Escape", code: "Escape" });
    expect(indicator()).toHaveAttribute("aria-expanded", "false");
    expect(indicator()).toHaveFocus();
    fireEvent.click(indicator());
    expect(indicator()).toHaveAttribute("aria-expanded", "true");
    fireEvent.pointerDown(document.body);
    expect(indicator()).toHaveAttribute("aria-expanded", "false");
  });
});
