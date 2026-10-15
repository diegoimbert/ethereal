import { afterEach, describe, expect, it } from "vitest";
import { act } from "@testing-library/react";
import { usePianoRollSection, resetPianoRollSection } from "@/features/piano-roll/section";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { itemSelection } from "@/timeline";
import { bindSelectionHistory } from "./selectionHistory";
import { resetTimeSelection, useTimeSelection } from "./store";

let unbind: (() => void) | null = null;
afterEach(() => {
  unbind?.();
  unbind = null;
  resetTimeSelection();
  resetPianoRollSection();
  resetStores();
});

describe("selection history", () => {
  it("undo brings back the sections of when the edit was made; redo those of when it was undone", async () => {
    const { mock } = await renderWithMock(<div />);
    unbind = bindSelectionHistory();
    const track = Object.values(useProjectStore.getState().project!.tracks).find((t) => t.kind !== "Master")!;
    const s1 = { start: 4, end: 8, tracks: [track.id] };
    const p1 = { clip: "clip", start: 1, end: 2 };
    act(() => {
      useTimeSelection.getState().setSelection(s1);
      usePianoRollSection.getState().setSection(p1);
    });
    await act(async () => void (await mock.send(cmd("Track", { type: "Rename", id: track.id, name: "Renamed" }))));
    // The view moves on after the edit.
    const s2 = { start: 12, end: 16, tracks: [track.id] };
    act(() => {
      useTimeSelection.getState().setSelection(s2);
      usePianoRollSection.getState().setSection(null);
    });

    await act(async () => void (await mock.send(cmd("Edit", { type: "Undo" }))));
    expect(useTimeSelection.getState().selection).toEqual(s1);
    expect(usePianoRollSection.getState().section).toEqual(p1);

    await act(async () => void (await mock.send(cmd("Edit", { type: "Redo" }))));
    expect(useTimeSelection.getState().selection).toEqual(s2);
    expect(usePianoRollSection.getState().section).toBeNull();
  });

  it("a section dropped by the gesture before its edit (a marker, or cleared) comes back on undo", async () => {
    const { mock } = await renderWithMock(<div />);
    unbind = bindSelectionHistory();
    const track = Object.values(useProjectStore.getState().project!.tracks).find((t) => t.kind !== "Master")!;
    const s1 = { start: 4, end: 8, tracks: [track.id] };
    const p1 = { clip: "clip", start: 1, end: 2 };
    act(() => {
      useTimeSelection.getState().setSelection(s1);
      usePianoRollSection.getState().setSection(p1);
    });
    // Pressing a clip turns the section into the insert marker; pressing a note clears the
    // piano-roll section; then the edit lands.
    act(() => {
      useTimeSelection.getState().setSelection({ start: 6, end: 6, tracks: [track.id] });
      usePianoRollSection.getState().setSection(null);
    });
    await act(async () => void (await mock.send(cmd("Track", { type: "Rename", id: track.id, name: "Moved" }))));

    await act(async () => void (await mock.send(cmd("Edit", { type: "Undo" }))));
    expect(useTimeSelection.getState().selection).toEqual(s1);
    expect(usePianoRollSection.getState().section).toEqual(p1);
  });

  it("undoing a duplicate selects the original notes again (the copies were selected)", async () => {
    const { mock } = await renderWithMock(<div />);
    unbind = bindSelectionHistory();
    const notes = Object.values(useProjectStore.getState().project!.notes).slice(0, 2);
    const originals = notes.map((n) => n.id);
    act(() => itemSelection.getState().select("note", originals, "replace"));
    await act(
      async () =>
        void (await mock.send(cmd("Note", { type: "Duplicate", copies: notes.map((n) => ({ from: n.id, new_id: `copy-${n.id}` })), offset: 4, transpose: 0 }))),
    );
    // The view selects the copies after the edit.
    act(() =>
      itemSelection.getState().select(
        "note",
        notes.map((n) => `copy-${n.id}`),
        "replace",
      ),
    );
    await act(async () => void (await mock.send(cmd("Edit", { type: "Undo" }))));
    expect([...itemSelection.getState().selected.note].sort()).toEqual([...originals].sort());
  });
});
