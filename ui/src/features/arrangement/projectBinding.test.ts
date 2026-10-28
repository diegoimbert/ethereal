// The time selection, the selected tracks and the clip clipboard follow the open project.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, ProjectId } from "@/generated";
import { useNotices } from "@/features/notifications";
import { resetTimeSelection, useTimeSelection } from "@/features/time-edits/store";
import { useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { MockTransport } from "@/transport/mock";
import { clearClipboard, copyClips, hasClipboard, pasteClips } from "./clipboard";
import { resetArrangementUi, useArrangementUi } from "./uiStore";

let mock: MockTransport;
let a: Project;

const store = () => useProjectStore.getState();
const trackIds = (p: Project) =>
  Object.values(p.tracks)
    .filter((t) => t.kind !== "Master")
    .map((t) => t.id);
/** `p` as another project (another id). */
const other = (p: Project): Project => ({ ...structuredClone(p), id: "019b0000-0000-7000-8000-000000000589" as ProjectId });
/** `p` without one track (as a collaborator's delete patches it). */
const without = (p: Project, track: string): Project => {
  const next = structuredClone(p);
  delete next.tracks[track];
  return next;
};

beforeEach(async () => {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  a = (await mock.connect())!;
  store().loadProject(a);
});

afterEach(() => {
  mock.dispose();
  store().reset();
  resetTimeSelection();
  resetArrangementUi();
  clearClipboard();
  itemSelection.getState().clear();
  useNotices.getState().clear();
});

describe("selections follow the open project (#589)", () => {
  it("opening another project clears the time selection, its cursor and the track focus", () => {
    const [t1, t2] = trackIds(a);
    useTimeSelection.getState().setSelection({ start: 4, end: 8, tracks: [t1!, t2!] });
    useArrangementUi.getState().setTrackSelection([t1!, t2!], t1!);
    expect(useTimeSelection.getState().cursor).toBe(4);

    store().loadProject(other(a));
    expect(useTimeSelection.getState().selection).toBeNull();
    expect(useTimeSelection.getState().cursor).toBe(0);
    expect(useArrangementUi.getState().trackFocus).toBeNull();
    expect(useArrangementUi.getState().selectedTracks.size).toBe(0);
  });

  it("a deleted track leaves the selections; the remaining tracks stay", () => {
    const [t1, t2] = trackIds(a);
    useTimeSelection.getState().setSelection({ start: 4, end: 8, tracks: [t1!, t2!] });
    useArrangementUi.getState().setTrackSelection([t1!, t2!], t2!);

    store().loadProject(without(a, t2!));
    expect(useTimeSelection.getState().selection).toEqual({ start: 4, end: 8, tracks: [t1] });
    expect([...useArrangementUi.getState().selectedTracks]).toEqual([t1]);
    expect(useArrangementUi.getState().trackFocus).toBeNull();

    // The last one goes too: nothing stays selected (an undone "add track" closes its inspector).
    store().loadProject(without(without(a, t2!), t1!));
    expect(useTimeSelection.getState().selection).toBeNull();
    expect(useArrangementUi.getState().selectedTracks.size).toBe(0);
  });

  it("clips copied in one project don't paste into another: paste is off, and says why", async () => {
    const clip = Object.values(a.clips)[0]!;
    itemSelection.getState().select("clip", [clip.id], "replace");
    expect(copyClips()).toBe(1);
    expect(hasClipboard()).toBe(true);

    store().loadProject(other(a));
    expect(hasClipboard()).toBe(false);
    const sent = vi.spyOn(mock, "send");
    expect(await pasteClips(mock, 0)).toEqual([]);
    expect(sent).not.toHaveBeenCalled();
    expect(useNotices.getState().notices.at(-1)?.message).toMatch(/another project/i);

    // Back in the project it was copied from: it pastes again.
    store().loadProject(a);
    expect(hasClipboard()).toBe(true);
  });
});

describe("clip paste is the copy as it was (ux-session-4)", () => {
  beforeEach(() => {
    mock.onEvent((e) => {
      if (e.type === "Patch") store().applyPatch(e.patch);
    });
  });

  it("notes and bounds changed after the copy don't follow into the paste", async () => {
    const clip = Object.values(a.clips).find((c) => c.content.type === "Midi" && Object.values(a.notes).some((n) => n.clip === c.id))!;
    const notesOf = (id: string) =>
      Object.values(store().project!.notes)
        .filter((n) => n.clip === id)
        .map((n) => [n.pitch, n.start, n.duration])
        .sort();
    const before = notesOf(clip.id);
    itemSelection.getState().select("clip", [clip.id], "replace");
    expect(copyClips()).toBe(1);
    // Edited after the copy: a note goes, the clip gets shorter.
    const note = Object.values(store().project!.notes).find((n) => n.clip === clip.id)!;
    await mock.send({ domain: "Note", command: { type: "Remove", ids: [note.id] } });
    await mock.send({ domain: "Clip", command: { type: "SetBounds", id: clip.id, start: clip.start, length: clip.length / 2, offset: clip.offset } });
    const [pasted] = await pasteClips(mock, 1000);
    expect(pasted).toBeDefined();
    const copy = store().project!.clips[pasted!]!;
    expect(copy.length).toBe(clip.length);
    expect(notesOf(pasted!)).toEqual(before);
  });

  it("an unchanged clip pastes as an exact duplicate", async () => {
    const clip = Object.values(a.clips)[0]!;
    itemSelection.getState().select("clip", [clip.id], "replace");
    copyClips();
    const sent = vi.spyOn(mock, "send");
    await pasteClips(mock, 1000);
    const commands = JSON.stringify(sent.mock.calls);
    expect(commands).toContain('"Duplicate"');
    expect(commands).not.toContain('"CreateMidi"');
  });
});
