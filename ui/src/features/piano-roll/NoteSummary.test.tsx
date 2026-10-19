import { act, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Note, NoteId } from "@/generated";
import { itemSelection, TempoMap } from "@/timeline";
import { NoteSummary } from "./NoteSummary";

const note = (id: string, pitch: number, start: number, duration: number, velocity: number, muted = false): Note =>
  ({ id: id as NoteId, clip: "c" as Note["clip"], pitch, velocity, release_velocity: 0.5, start, duration, muted }) as Note;

afterEach(() => itemSelection.getState().clear());

describe("NoteSummary", () => {
  it("lists the notes in time order and announces the selection", () => {
    const notes = [note("b", 64, 4, 2, 0.5, true), note("a", 60, 0, 1, 1)];
    render(<NoteSummary notes={notes} tempo={TempoMap.constant()} />);
    const list = screen.getByRole("list", { name: "2 notes" });
    expect(
      within(list)
        .getAllByRole("listitem")
        .map((li) => li.textContent),
    ).toEqual(["C3 at 1.1.1, length 0.1.0, velocity 127", "E3 at 2.1.1, length 0.2.0, velocity 64, muted"]);
    expect(screen.getByRole("status")).toHaveTextContent("");
    act(() => itemSelection.getState().select("note", ["a" as NoteId], "replace"));
    expect(screen.getByRole("status")).toHaveTextContent("selected note C3 at 1.1.1, length 0.1.0, velocity 127");
    act(() => itemSelection.getState().select("note", ["a" as NoteId, "b" as NoteId], "replace"));
    expect(screen.getByRole("status")).toHaveTextContent("2 notes selected");
  });
});
