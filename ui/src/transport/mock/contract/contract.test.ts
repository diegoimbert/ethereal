/**
 * Mock/controller contract (#596): UI tests run on `MockTransport`, so it must do what the
 * controller does for the same commands. Each scenario here has a twin in
 * `crates/ether-controller/tests/mock_contract.rs`; the transport one is shared data
 * (`transport.json`, read by both).
 */
import { afterEach, describe, expect, it } from "vitest";
import type { Command, Event, Project, ProjectSummary, TrackId, TransportState } from "@/generated";
import { cmd } from "../../cmd";
import { newId, newProjectId } from "../../ids";
import { MockTransport } from "../MockTransport";
import contract from "./transport.json";

let mock: MockTransport;
afterEach(() => mock?.dispose());

async function connect(): Promise<{ events: Event[]; project: () => Project; transport: () => TransportState; position: () => number }> {
  mock = new MockTransport({ timers: "manual", seed: 9 });
  const events: Event[] = [];
  let project: Project | null = null;
  let transport: TransportState | null = null;
  let position = 0;
  mock.onEvent((e) => {
    events.push(e);
    if (e.type === "ProjectLoaded") project = e.project;
    if (e.type === "Transport") transport = e.state;
  });
  mock.subscribePlayhead((f) => (position = f.transport.position));
  project = await mock.connect();
  return { events, project: () => project!, transport: () => transport!, position: () => position };
}

const send = (c: Command) => mock.send(c);
/** The engine's document now (events only carry patches). */
async function current(): Promise<Project> {
  const r = await send(cmd("Project", { type: "Get" }));
  if (r.type !== "Project") throw new Error(r.type);
  return r.project;
}
const midiTrack = (p: Project): TrackId => Object.values(p.tracks).find((t) => t.kind === "Midi")!.id;

describe("mock/controller contract (#596)", () => {
  it("play resumes where the playhead is; a locate moves the start position only while stopped", async () => {
    const s = await connect();
    for (const [i, step] of contract.steps.entries()) {
      await send(step.command as Command);
      const t = s.transport();
      expect({ step: i, playing: t.playing, start_position: t.start_position, position: s.position() }).toEqual({
        step: i,
        playing: step.playing,
        start_position: step.start_position,
        position: step.position,
      });
    }
  });

  it("save as renames the open document in place: playback, armed tracks and the playhead go on", async () => {
    const s = await connect();
    const track = midiTrack(s.project());
    await send(cmd("Recording", { type: "Arm", track, armed: true, exclusive: false }));
    await send(cmd("Transport", { type: "Locate", position: 8 }));
    await send(cmd("Transport", { type: "Play" }));
    await send(cmd("Track", { type: "Rename", id: track, name: "Lead" }));
    const from = s.events.length;
    const id = newProjectId();
    const reply = await send(cmd("Project", { type: "SaveAs", new_id: id, name: "Copy" }));
    expect(reply.type === "Project" && reply.project.id).toBe(id);
    const after = s.events.slice(from);
    expect(after.some((e) => e.type === "ProjectLoaded" && e.project.id === id && e.project.settings.name === "Copy")).toBe(true);
    const saved = after.find((e): e is Extract<Event, { type: "Project" }> => e.type === "Project" && e.event.type === "Saved");
    expect((saved?.event as { project: ProjectSummary } | undefined)?.project.id).toBe(id);
    // Nothing stopped, rewound or disarmed; the copy has the unsaved rename.
    expect(after.some((e) => e.type === "Recording")).toBe(false);
    expect(s.transport().playing).toBe(true);
    expect(s.position()).toBe(8);
    expect((await current()).tracks[track]!.name).toBe("Lead");
    // The history starts over with the copy.
    const h = await send(cmd("History", { type: "State" }));
    expect(h.type === "HistoryState" && h.history.can_undo).toBe(false);
  });

  it("duplicating the open project copies its current (unsaved) state", async () => {
    const s = await connect();
    const p = s.project();
    await send(cmd("Project", { type: "Save" }));
    const track = newId();
    await send(cmd("Track", { type: "Create", id: track, kind: "Midi", name: "Unsaved", color: null, parent: null, before: null }));
    const copy = newProjectId();
    await send(cmd("Project", { type: "Duplicate", id: p.id, new_id: copy, name: "Dup" }));
    await send(cmd("Project", { type: "Open", id: copy }));
    expect(Object.values((await current()).tracks).some((t) => t.name === "Unsaved")).toBe(true);
  });

  it("a duplicated track's clip envelopes target the copy", async () => {
    const s = await connect();
    const track = midiTrack(s.project());
    const clip = newId();
    await send(cmd("Clip", { type: "CreateMidi", id: clip, track, start: 0, length: 4, name: "C" }));
    const lane = newId();
    await send(cmd("Automation", { type: "CreateLane", id: lane, owner: { type: "Clip", clip }, target: { type: "TrackVolume", track } }));
    const copy = newId();
    await send(cmd("Track", { type: "Duplicate", id: track, new_id: copy }));
    const p = await current();
    const copied = new Set(Object.values(p.clips).flatMap((c) => (c.track === copy ? [c.id] : [])));
    const lanes = Object.values(p.automation_lanes).filter((l) => l.owner.type === "Clip" && copied.has(l.owner.clip));
    expect(lanes.map((l) => l.target)).toEqual([{ type: "TrackVolume", track: copy }]);
  });
});
