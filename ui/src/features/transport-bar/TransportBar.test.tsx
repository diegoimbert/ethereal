import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { tempoPoints, useProjectStore } from "@/state";
import { renderWithMock, resetStores } from "./testUtils";
import { TransportBar } from "./index";

const store = () => useProjectStore.getState();

afterEach(resetStores);

describe("TransportBar", () => {
  it("renders disabled without an engine", () => {
    render(<TransportBar />);
    expect(screen.getByRole("button", { name: "Play" })).toBeDisabled();
    expect(screen.getByTestId("engine-status")).toHaveAttribute("data-level", "off");
  });

  it("plays, stops and shows the position", async () => {
    const { mock } = await renderWithMock(<TransportBar />);
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    // 120 BPM: 1 s = 2 beats = bar 1, beat 3.
    act(() => mock.tick(1000));
    expect(screen.getByTestId("position-bars").textContent).toBe("1.3.1");
    expect(screen.getByTestId("position-time").textContent).toMatch(/^0:01\.0/);

    fireEvent.click(screen.getByRole("button", { name: "Stop", pressed: true }));
    await waitFor(() => expect(store().transport?.playing).toBe(false));
  });

  it("toggles play with Space, but not while typing", async () => {
    await renderWithMock(<TransportBar />);
    fireEvent.keyDown(window, { key: " " });
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    fireEvent.keyDown(screen.getByLabelText("Tempo"), { key: " " });
    await act(() => Promise.resolve());
    expect(store().transport?.playing).toBe(true);
  });

  it("toggles loop and metronome", async () => {
    await renderWithMock(<TransportBar />);
    const loop = screen.getByRole("button", { name: "Loop" });
    const before = store().transport!.loop_enabled;
    fireEvent.click(loop);
    await waitFor(() => expect(store().transport!.loop_enabled).toBe(!before));
    expect(loop).toHaveAttribute("aria-pressed", String(!before));

    fireEvent.click(screen.getByRole("button", { name: "Metronome" }));
    await waitFor(() => expect(store().transport!.metronome).toBe(true));
  });

  it("edits tempo, rejects invalid values, and undoes", async () => {
    await renderWithMock(<TransportBar />);
    const tempo = screen.getByLabelText<HTMLInputElement>("Tempo");
    fireEvent.focus(tempo);
    fireEvent.change(tempo, { target: { value: "128" } });
    fireEvent.keyDown(tempo, { key: "Enter" });
    await waitFor(() => expect(store().transport!.bpm).toBe(128));
    expect(tempoPoints(store().project!)[0]!.bpm).toBe(128);
    expect(tempo.value).toBe("128.00");

    fireEvent.focus(tempo);
    fireEvent.change(tempo, { target: { value: "abc" } });
    fireEvent.blur(tempo);
    expect(tempo).toHaveAttribute("aria-invalid", "true");
    expect(tempo.value).toBe("128.00");

    fireEvent.keyDown(tempo, { key: "ArrowUp" });
    await waitFor(() => expect(store().transport!.bpm).toBe(129));

    const undo = screen.getByRole("button", { name: "Undo" });
    await waitFor(() => expect(undo).toBeEnabled());
    fireEvent.click(undo);
    await waitFor(() => expect(store().transport!.bpm).toBe(128));
    fireEvent.click(screen.getByRole("button", { name: "Redo" }));
    await waitFor(() => expect(store().transport!.bpm).toBe(129));
  });

  it("undoes with Ctrl+Z", async () => {
    await renderWithMock(<TransportBar />);
    fireEvent.click(screen.getByRole("button", { name: "Metronome" }));
    await waitFor(() => expect(store().history.can_undo).toBe(true));
    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    await waitFor(() => expect(store().transport!.metronome).toBe(false));
  });

  it("sets the time signature", async () => {
    await renderWithMock(<TransportBar />);
    const field = screen.getByLabelText<HTMLInputElement>("Time signature");
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "7/8" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(store().transport!.time_signature).toEqual({ numerator: 7, denominator: 8 }));
    expect(field.value).toBe("7/8");
  });

  it("shows engine errors (record is unsupported by the mock)", async () => {
    await renderWithMock(<TransportBar />);
    fireEvent.click(screen.getByRole("button", { name: "Record" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/recording/);
    fireEvent.click(alert);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows CPU load and engine status", async () => {
    const { mock } = await renderWithMock(<TransportBar />);
    await waitFor(() => expect(screen.getByTestId("engine-status")).toHaveAttribute("data-level", "ok"));
    expect(screen.getByTestId("engine-status").getAttribute("title")).toMatch(/48000 Hz/);
    fireEvent.click(screen.getByRole("button", { name: "Play" }));
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    act(() => mock.tick(200));
    expect(screen.getByTestId("cpu").textContent).toMatch(/^CPU \d+%$/);
  });
});
