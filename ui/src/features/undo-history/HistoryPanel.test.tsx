import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { playheadStore, useProjectStore } from "@/state";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { HistoryPanel, nameCurrentCheckpoint, useUndoHistoryUi } from "./index";

const store = () => useProjectStore.getState();
let seq = 0;
const newId = () => `01K${String(++seq).padStart(23, "0")}`;

async function setup() {
  const mock = new MockTransport({ timers: "manual", seed: 3 });
  const utils = render(
    <TransportProvider transport={mock}>
      <HistoryPanel />
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!store().project) throw new Error("not connected");
  });
  await screen.findByRole("list", { name: "Undo history" });
  return { mock, ...utils };
}

async function addTrack(mock: MockTransport, name: string): Promise<string> {
  const id = newId();
  await act(() => mock.send(cmd("Track", { type: "Create", id, kind: "Midi", name, color: null, parent: null, before: null })));
  return id;
}

const rows = () => within(screen.getByRole("list", { name: "Undo history" })).getAllByRole("listitem");
const current = () => screen.getAllByRole("button").find((b) => b.getAttribute("aria-current") === "step");

afterEach(() => {
  store().reset();
  playheadStore.reset();
  useUndoHistoryUi.setState({ editing: null, editCurrent: false });
});

describe("HistoryPanel", () => {
  it("lists the edits as a timeline and follows new ones", async () => {
    const { mock } = await setup();
    expect(rows()).toHaveLength(1);
    expect(screen.getByText("Project opened")).toBeTruthy();
    expect(screen.getByText("Edits you make appear here.")).toBeTruthy();
    await addTrack(mock, "Bass");
    await addTrack(mock, "Keys");
    await waitFor(() => expect(rows()).toHaveLength(3));
    expect(current()?.textContent).toContain(rows()[2]!.textContent!.slice(0, 5));
    expect(screen.getByText("2 steps")).toBeTruthy();
  });

  it("jumps back and forward by clicking a step", async () => {
    const { mock } = await setup();
    const bass = await addTrack(mock, "Bass");
    const keys = await addTrack(mock, "Keys");
    await waitFor(() => expect(rows()).toHaveLength(3));

    // Back to before everything.
    fireEvent.click(within(rows()[0]!).getByRole("button"));
    await waitFor(() => expect(store().project!.tracks[bass]).toBeUndefined());
    expect(store().project!.tracks[keys]).toBeUndefined();
    await waitFor(() => expect(rows()[1]!.className).toContain("eth-history__row--undone"));
    expect(screen.getByText("2 steps · 2 undone")).toBeTruthy();

    // Forward to the first step.
    fireEvent.click(within(rows()[1]!).getAllByRole("button")[0]!);
    await waitFor(() => expect(store().project!.tracks[bass]).toBeTruthy());
    expect(store().project!.tracks[keys]).toBeUndefined();
    await waitFor(() => expect(rows()[1]!.className).toContain("eth-history__row--current"));
  });

  it("names, renames and removes checkpoints", async () => {
    const { mock } = await setup();
    await addTrack(mock, "Bass");
    await waitFor(() => expect(rows()).toHaveLength(2));

    fireEvent.click(within(rows()[1]!).getByRole("button", { name: "Name checkpoint" }));
    const input = screen.getByRole("textbox", { name: /Checkpoint name/ });
    fireEvent.change(input, { target: { value: "Verse done" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(screen.getByText("Verse done")).toBeTruthy());
    expect(rows()[1]!.className).toContain("eth-history__row--checkpoint");

    // Escape cancels a rename.
    fireEvent.click(within(rows()[1]!).getByRole("button", { name: "Rename checkpoint" }));
    const again = screen.getByRole("textbox", { name: /Checkpoint name/ });
    fireEvent.change(again, { target: { value: "Nope" } });
    fireEvent.keyDown(again, { key: "Escape" });
    expect(screen.getByText("Verse done")).toBeTruthy();

    // A blank name removes it.
    fireEvent.click(within(rows()[1]!).getByRole("button", { name: "Rename checkpoint" }));
    const blank = screen.getByRole("textbox", { name: /Checkpoint name/ });
    fireEvent.change(blank, { target: { value: "  " } });
    fireEvent.blur(blank);
    await waitFor(() => expect(screen.queryByText("Verse done")).toBeNull());
  });

  it("names the current step from the palette", async () => {
    const { mock } = await setup();
    await addTrack(mock, "Bass");
    await waitFor(() => expect(rows()).toHaveLength(2));
    act(() => nameCurrentCheckpoint());
    const input = await screen.findByRole("textbox", { name: /Checkpoint name/ });
    expect(rows()[1]!.contains(input)).toBe(true);
  });
});
