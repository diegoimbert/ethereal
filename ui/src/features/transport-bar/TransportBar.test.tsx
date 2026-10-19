import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { t } from "@/i18n";
import { tempoPoints, useProjectStore } from "@/state";
import { useNotices } from "@/features/notifications";
import type { Event } from "@/generated";
import { cmd, CommandFailedError } from "@/transport";
import { renderWithMock, resetStores } from "./testUtils";
import { TransportBar } from "./index";

const store = () => useProjectStore.getState();

afterEach(resetStores);

describe("TransportBar", () => {
  it("renders disabled without an engine", () => {
    render(<TransportBar />);
    expect(screen.getByRole("button", { name: "play" })).toBeDisabled();
    expect(screen.getByTestId("engine-status")).toHaveAttribute("data-level", "off");
  });

  it("plays, stops and shows the position", async () => {
    const { mock } = await renderWithMock(<TransportBar />);
    fireEvent.click(screen.getByRole("button", { name: "play" }));
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    // 120 BPM: 1 s = 2 beats = bar 1, beat 3.
    act(() => mock.tick(1000));
    expect(screen.getByTestId("position-bars").textContent).toBe("1.3.1");
    expect(screen.getByTestId("position-time").textContent).toMatch(/^0:01\.0/);

    fireEvent.click(screen.getByRole("button", { name: "stop", pressed: true }));
    await waitFor(() => expect(store().transport?.playing).toBe(false));
  });

  it("toggles play with Space, but not while typing", async () => {
    await renderWithMock(<TransportBar />);
    fireEvent.keyDown(window, { key: " " });
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    fireEvent.keyDown(screen.getByLabelText("tempo"), { key: " " });
    await act(() => Promise.resolve());
    expect(store().transport?.playing).toBe(true);
  });

  it("toggles loop and metronome", async () => {
    await renderWithMock(<TransportBar />);
    const loop = screen.getByRole("button", { name: "loop" });
    const before = store().transport!.loop_enabled;
    fireEvent.click(loop);
    await waitFor(() => expect(store().transport!.loop_enabled).toBe(!before));
    // The store can update before React re-renders: assert on the DOM with waitFor too.
    await waitFor(() => expect(loop).toHaveAttribute("aria-pressed", String(!before)));

    fireEvent.click(screen.getByRole("button", { name: "metronome" }));
    await waitFor(() => expect(store().transport!.metronome).toBe(true));
  });

  it("drags the tempo up and down as one undo step; a plain click edits", async () => {
    await renderWithMock(<TransportBar />);
    const tempo = screen.getByLabelText<HTMLInputElement>("tempo");
    await waitFor(() => expect(tempo).toBeEnabled());
    fireEvent.pointerDown(tempo, { button: 0, pointerId: 1, clientY: 100 });
    fireEvent.pointerMove(tempo, { pointerId: 1, clientY: 90 });
    fireEvent.pointerMove(tempo, { pointerId: 1, clientY: 80 });
    await waitFor(() => expect(store().transport!.bpm).toBe(130));
    fireEvent.pointerMove(tempo, { pointerId: 1, clientY: 110 });
    await waitFor(() => expect(store().transport!.bpm).toBe(115));
    fireEvent.pointerUp(tempo, { pointerId: 1, clientY: 110 });
    expect(document.activeElement).not.toBe(tempo);

    const undo = screen.getByRole("button", { name: "undo" });
    await waitFor(() => expect(undo).toBeEnabled());
    fireEvent.click(undo);
    await waitFor(() => expect(store().transport!.bpm).toBe(120));

    fireEvent.pointerDown(tempo, { button: 0, pointerId: 2, clientY: 100 });
    fireEvent.pointerUp(tempo, { pointerId: 2, clientY: 100 });
    expect(document.activeElement).toBe(tempo);
  });

  it("edits tempo, rejects invalid values, and undoes", async () => {
    await renderWithMock(<TransportBar />);
    const tempo = screen.getByLabelText<HTMLInputElement>("tempo");
    fireEvent.focus(tempo);
    fireEvent.change(tempo, { target: { value: "128" } });
    fireEvent.keyDown(tempo, { key: "Enter" });
    await waitFor(() => expect(store().transport!.bpm).toBe(128));
    expect(tempoPoints(store().project!)[0]!.bpm).toBe(128);
    await waitFor(() => expect(tempo.value).toBe("128.00"));

    fireEvent.focus(tempo);
    fireEvent.change(tempo, { target: { value: "abc" } });
    fireEvent.blur(tempo);
    expect(tempo).toHaveAttribute("aria-invalid", "true");
    expect(tempo.value).toBe("128.00");

    fireEvent.keyDown(tempo, { key: "ArrowUp" });
    await waitFor(() => expect(store().transport!.bpm).toBe(129));

    const undo = screen.getByRole("button", { name: "undo" });
    await waitFor(() => expect(undo).toBeEnabled());
    fireEvent.click(undo);
    await waitFor(() => expect(store().transport!.bpm).toBe(128));
    const redo = screen.getByRole("button", { name: "redo" });
    await waitFor(() => expect(redo).toBeEnabled());
    fireEvent.click(redo);
    await waitFor(() => expect(store().transport!.bpm).toBe(129));
  });

  it("undoes with Ctrl+Z", async () => {
    await renderWithMock(<TransportBar />);
    fireEvent.click(screen.getByRole("button", { name: "metronome" }));
    await waitFor(() => expect(store().history.can_undo).toBe(true));
    fireEvent.keyDown(window, { key: "z", ctrlKey: true });
    await waitFor(() => expect(store().transport!.metronome).toBe(false));
  });

  it("sets the time signature", async () => {
    await renderWithMock(<TransportBar />);
    const field = screen.getByLabelText<HTMLInputElement>("time signature");
    fireEvent.focus(field);
    fireEvent.change(field, { target: { value: "7/8" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(store().transport!.time_signature).toEqual({ numerator: 7, denominator: 8 }));
    await waitFor(() => expect(field.value).toBe("7/8"));
  });

  it("shows engine errors", async () => {
    const { mock } = await renderWithMock(<TransportBar />);
    const command = cmd("Recording", { type: "SetRecording", enabled: true });
    vi.spyOn(mock, "send").mockRejectedValueOnce(new CommandFailedError({ code: "Unsupported", message: "recording is not available" }, command));
    fireEvent.click(screen.getByRole("button", { name: "record" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/recording/);
    fireEvent.click(within(alert).getByRole("button"));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows CPU load and engine status", async () => {
    const { mock } = await renderWithMock(<TransportBar />);
    await waitFor(() => expect(screen.getByTestId("engine-status")).toHaveAttribute("data-level", "ok"));
    expect(screen.getByTestId("engine-status").getAttribute("title")).toMatch(/48000 hz/);
    fireEvent.click(screen.getByRole("button", { name: "play" }));
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    act(() => mock.tick(200));
    expect(screen.getByTestId("cpu").textContent).toMatch(/^cpu \d+%$/);
  });

  it("shows the audio device closed while no local engine plays", async () => {
    const { mock } = await renderWithMock(<TransportBar />);
    const indicator = () => screen.getByTestId("engine-status");
    await waitFor(() => expect(indicator()).toHaveAttribute("data-level", "ok"));
    const status = { running: true, backend: "null", sample_rate: 48000, buffer_size: 256, output_latency: 256, input_latency: 0, xruns: 0, instance: "mock" };
    const emit = (e: Event) => act(() => (mock as unknown as { emit(e: Event): void }).emit(e));
    emit({ type: "Engine", event: { type: "Status", status: { ...status, device_released: true } } });
    expect(indicator()).toHaveAttribute("data-level", "off");
    expect(indicator()).toHaveAttribute("title", "audio device closed (no local engine)");
    expect(indicator()).toHaveTextContent("closed");
    emit({ type: "Engine", event: { type: "Status", status: { ...status, backend: "cpal", device_released: false } } });
    expect(indicator()).toHaveAttribute("data-level", "ok");
    expect(indicator()).toHaveTextContent("cpal");
  });

  it("shows disk-streaming dropouts, toasts once per playback session and resets on click", async () => {
    const { mock } = await renderWithMock(<TransportBar />);
    const emit = (e: Event) => act(() => (mock as unknown as { emit(e: Event): void }).emit(e));
    const underruns = (count: number) => emit({ type: "Media", event: { type: "StreamUnderruns", count } });
    const diskToasts = () => useNotices.getState().notices.filter((n) => n.title === t("shell.transport.disk_toast_title"));
    useNotices.getState().clear();
    // Nothing streamed out of time: no light (hosts that don't stream never show one).
    expect(screen.queryByTestId("disk-dropouts")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "play" }));
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    underruns(3);
    const light = screen.getByTestId("disk-dropouts");
    expect(light.textContent).toBe("disk 3");
    expect(light.getAttribute("title")).toMatch(/^disk too slow: 3 dropouts since .+\. click to reset\.$/);
    expect(diskToasts()).toHaveLength(1);
    // Later reports in the same session add up, without another toast (even once dismissed).
    underruns(2);
    expect(screen.getByTestId("disk-dropouts").textContent).toBe("disk 5");
    useNotices.getState().clear();
    underruns(1);
    expect(diskToasts()).toHaveLength(0);

    // A new playback session toasts again on its first dropout.
    fireEvent.click(screen.getByRole("button", { name: "stop", pressed: true }));
    await waitFor(() => expect(store().transport?.playing).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "play" }));
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    underruns(1);
    expect(diskToasts()).toHaveLength(1);
    expect(screen.getByTestId("disk-dropouts").textContent).toBe("disk 7");

    fireEvent.click(screen.getByTestId("disk-dropouts"));
    expect(screen.queryByTestId("disk-dropouts")).toBeNull();
    useNotices.getState().clear();
  });
});
