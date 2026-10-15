import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, Event, Project, ReplyValue, Track } from "@/generated";
import { useConfirm, type ContextMenuEntry, type ContextMenuItem } from "@/kit";
import { useProjectStore } from "@/state";
import { createDemoProject, type EngineTransport } from "@/transport";
import { resetTimeSelection, timeSelectionMenu, useTimeSelection } from "@/features/time-edits";
import { FreezeHeaderStatus } from "./FreezeHeaderStatus";
import { descendantsOf, frozenAncestor, isFrozenTrack } from "./inert";
import { freezeClipEntries, freezeTrackEntries, withFreezeTrackEntries } from "./menus";
import { resetRenderJobs } from "./store";

/** A transport that records commands and lets the test push events / choose replies. */
class FakeTransport {
  readonly kind = "mock" as const;
  sent: Command[] = [];
  reply: (c: Command) => ReplyValue = (c) =>
    c.domain === "Freeze" && "job" in c.command && c.command.type !== "Cancel" ? { type: "RenderStarted", job: c.command.job } : { type: "Unit" };
  private listeners = new Set<(e: Event) => void>();
  connect = () => Promise.reject(new Error("unused"));
  send = async (c: Command): Promise<ReplyValue> => {
    this.sent.push(c);
    return this.reply(c);
  };
  onEvent = (l: (e: Event) => void) => {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  };
  emit(e: Event) {
    for (const l of this.listeners) l(e);
  }
  subscribePlayhead = () => () => {};
  subscribeMeters = () => () => {};
  dispose = () => {};
}

const items = (entries: ContextMenuEntry[]) => entries.filter((e): e is ContextMenuItem => e !== "separator");
const labels = (entries: ContextMenuEntry[]) => entries.map((e) => (e === "separator" ? "—" : e.label));
const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

let project: Project;
let fake: FakeTransport;
let transport: EngineTransport;

const trackOf = (kind: Track["kind"]) =>
  Object.values(project.tracks).find((t) => t.kind === kind && Object.values(project.clips).some((c) => c.track === t.id))!;

beforeEach(() => {
  project = createDemoProject();
  useProjectStore.setState({ project });
  resetRenderJobs();
  fake = new FakeTransport();
  transport = fake as unknown as EngineTransport;
});

afterEach(() => {
  useProjectStore.setState({ project: null });
  useConfirm.getState().close();
});

describe("freeze menus", () => {
  it("offers Freeze before the destructive group, then Unfreeze / Flatten when frozen", () => {
    const t = trackOf("Midi");
    const menu = withFreezeTrackEntries(["separator", { label: "delete track", onSelect: () => {} }], transport, t, [t.id]);
    expect(labels(menu)).toEqual(["—", "freeze track", "—", "delete track"]);
    const frozen = { ...t, freeze: { media: "m", start: 0 } };
    expect(labels(freezeTrackEntries(transport, frozen, []))).toEqual(["unfreeze track", "flatten track"]);
    expect(freezeTrackEntries(transport, { ...t, kind: "Return" }, [])).toEqual([]);
  });

  it("flatten asks first, naming the tracks; cancelling sends nothing (#717)", async () => {
    const a = trackOf("Midi");
    const b = trackOf("Audio");
    const fa = { ...a, freeze: { media: "m", start: 0 } };
    const fb = { ...b, freeze: { media: "m", start: 0 } };
    project = { ...project, tracks: { ...project.tracks, [a.id]: fa, [b.id]: fb } };
    useProjectStore.setState({ project });
    const flatten = items(freezeTrackEntries(transport, fa, [a.id, b.id])).find((e) => e.label === "flatten 2 tracks")!;
    flatten.onSelect();
    const ask = useConfirm.getState().request!;
    expect(ask).toMatchObject({ title: "flatten 2 tracks?", action: "flatten", danger: true });
    expect(ask.body).toMatch(/only undo brings them back/);
    act(() => useConfirm.getState().close());
    await flush();
    expect(fake.sent).toEqual([]);
    flatten.onSelect();
    act(() => useConfirm.getState().request!.run());
    await flush();
    expect(fake.sent.map((c) => (c.command as { type: string; track: string }).track).sort()).toEqual([a.id, b.id].sort());
  });

  it("offers bounce and consolidate on clips (in place for audio only)", () => {
    const midi = Object.values(project.clips).find((c) => c.content.type === "Midi")!;
    const audio = Object.values(project.clips).find((c) => c.content.type === "Audio")!;
    expect(labels(freezeClipEntries(transport, midi))).toEqual(["bounce to new track", "consolidate"]);
    expect(labels(freezeClipEntries(transport, audio))).toEqual(["bounce to new track", "bounce in place", "consolidate"]);
  });

  it("offers Bounce to New Track on a time selection, before Delete Time", async () => {
    const midi = trackOf("Midi");
    const audio = trackOf("Audio");
    const [cm, ca] = [midi, audio].map((t) => Object.values(project.clips).find((c) => c.track === t.id && !c.lane)!);
    const start = Math.min(cm!.start, ca!.start) + 0.25;
    const end = Math.max(cm!.start + cm!.length, ca!.start + ca!.length) - 0.25;
    act(() => useTimeSelection.getState().setSelection({ start, end, tracks: [midi.id, audio.id] }));
    const menu = timeSelectionMenu(transport);
    const l = labels(menu);
    expect(l.indexOf("bounce to new track")).toBeGreaterThan(l.indexOf("insert silence"));
    expect(l.indexOf("bounce to new track")).toBeLessThan(l.indexOf("delete time"));
    const entry = items(menu).find((e) => e.label === "bounce to new track")!;
    expect(entry.disabled).toBe(false);
    entry.onSelect();
    await flush();
    expect(fake.sent.at(-1)).toMatchObject({ domain: "Freeze", command: { type: "BounceRange", start, end } });
    expect((fake.sent.at(-1)!.command as { tracks: string[] }).tracks.sort()).toEqual([midi.id, audio.id].sort());

    // Nothing in the range: shown, disabled.
    act(() => useTimeSelection.getState().setSelection({ start: 9000, end: 9004, tracks: [midi.id] }));
    expect(items(timeSelectionMenu(transport)).find((e) => e.label === "bounce to new track")!.disabled).toBe(true);
    act(() => resetTimeSelection());
  });
});

describe("render jobs", () => {
  it("freezes with progress in the header, then shows the frozen toggle", async () => {
    const t = trackOf("Midi");
    const { rerender } = render(<FreezeHeaderStatus track={t} transport={transport} />);
    items(freezeTrackEntries(transport, t, [t.id]))[0]!.onSelect();
    await flush();
    const sent = fake.sent.at(-1)!;
    expect(sent.domain).toBe("Freeze");
    const job = (sent.command as { job: string }).job;
    act(() => fake.emit({ type: "Freeze", event: { type: "Progress", job, progress: 0.4 } }));
    expect(screen.getByRole("progressbar", { name: `freezing ${t.name}` })).toHaveAttribute("aria-valuenow", "40");
    // While it runs, the menu offers Cancel.
    expect(labels(freezeTrackEntries(transport, t, []))).toEqual(["cancel freeze"]);
    act(() => fake.emit({ type: "Freeze", event: { type: "Done", job } }));
    expect(screen.queryByRole("progressbar")).toBeNull();
    rerender(<FreezeHeaderStatus track={{ ...t, freeze: { media: "m", start: 0 } }} transport={transport} />);
    fireEvent.click(screen.getByRole("button", { name: `unfreeze ${t.name}` }));
    await flush();
    expect(fake.sent.at(-1)).toEqual({ domain: "Freeze", command: { type: "Unfreeze", track: t.id } });
  });

  it("queues a second job until the first ends, and shows failures", async () => {
    const midi = trackOf("Midi");
    const audio = trackOf("Audio");
    render(
      <>
        <FreezeHeaderStatus track={midi} transport={transport} />
        <FreezeHeaderStatus track={audio} transport={transport} />
      </>,
    );
    items(freezeTrackEntries(transport, midi, []))[0]!.onSelect();
    items(freezeTrackEntries(transport, audio, []))[0]!.onSelect();
    await flush();
    expect(fake.sent).toHaveLength(1);
    expect(screen.getByRole("progressbar", { name: `freezing ${audio.name}` })).not.toHaveAttribute("aria-valuenow");
    const job = (fake.sent[0]!.command as { job: string }).job;
    act(() => fake.emit({ type: "Freeze", event: { type: "Failed", job, message: "disk full" } }));
    await flush();
    expect(fake.sent).toHaveLength(2);
    const warning = screen.getByRole("button", { name: "render failed: disk full" });
    fireEvent.click(warning);
    expect(screen.queryByRole("button", { name: /render failed/ })).toBeNull();
  });

  it("cancels a running job", async () => {
    const t = trackOf("Midi");
    render(<FreezeHeaderStatus track={t} transport={transport} />);
    items(freezeTrackEntries(transport, t, []))[0]!.onSelect();
    await flush();
    const job = (fake.sent[0]!.command as { job: string }).job;
    fireEvent.click(screen.getByRole("button", { name: `cancel freeze of ${t.name}` }));
    await flush();
    expect(fake.sent.at(-1)).toEqual({ domain: "Freeze", command: { type: "Cancel", job } });
    act(() => fake.emit({ type: "Freeze", event: { type: "Cancelled", job } }));
    expect(screen.queryByRole("progressbar")).toBeNull();
  });
});

describe("freeze group", () => {
  /** The demo project with its Midi and Audio tracks moved into a new group. */
  function withGroup(frozen: boolean) {
    const midi = trackOf("Midi");
    const audio = trackOf("Audio");
    const group: Track = {
      ...midi,
      id: "01JXT21Q00TCH1Q3BRJMTQCGRP" as Track["id"],
      kind: "Group",
      name: "Band",
      parent: null,
      freeze: frozen ? { media: "m" as never, start: 0 } : null,
    } as Track;
    const tracks = {
      ...project.tracks,
      [group.id]: group,
      [midi.id]: { ...midi, parent: group.id },
      [audio.id]: { ...audio, parent: group.id },
    };
    project = { ...project, tracks };
    useProjectStore.setState({ project });
    return { group, midi: project.tracks[midi.id]!, audio: project.tracks[audio.id]! };
  }

  it("finds the frozen ancestor and the descendants", () => {
    const { group, midi } = withGroup(true);
    expect(frozenAncestor(project.tracks, midi.id)?.id).toBe(group.id);
    expect(frozenAncestor(project.tracks, group.id)).toBeUndefined();
    expect(isFrozenTrack(project, midi)).toBe(true);
    expect(
      descendantsOf(project.tracks, group.id)
        .map((t) => t.id)
        .sort(),
    ).toEqual([midi.id, trackOf("Audio").id].sort());
  });

  it("offers Freeze Group, then Unfreeze Group and Flatten Group", async () => {
    const { group } = withGroup(false);
    const menu = withFreezeTrackEntries(["separator", { label: "delete track", onSelect: () => {} }], transport, group, [group.id]);
    expect(labels(menu)).toEqual(["—", "freeze group", "—", "delete track"]);
    items(freezeTrackEntries(transport, group, [group.id]))[0]!.onSelect();
    await flush();
    const sent = fake.sent.at(-1)!;
    expect(sent).toMatchObject({ domain: "Freeze", command: { type: "Freeze", track: group.id } });
    resetRenderJobs();
    const frozen = withGroup(true).group;
    expect(labels(freezeTrackEntries(transport, frozen, []))).toEqual(["unfreeze group", "flatten group"]);
    const before = fake.sent.length;
    items(freezeTrackEntries(transport, frozen, []))[1]!.onSelect();
    await flush();
    // #717: flatten asks first.
    const ask = useConfirm.getState().request!;
    expect(ask.title).toBe(`flatten “${frozen.name}”?`);
    expect(ask.body).toMatch(/tracks inside a group/);
    expect(fake.sent.length).toBe(before);
    act(() => ask.run());
    await flush();
    expect(fake.sent.at(-1)).toMatchObject({ domain: "Freeze", command: { type: "Flatten", track: frozen.id } });
  });

  it("disables Freeze Group for a group without clips", () => {
    const { group } = withGroup(false);
    project = { ...project, clips: {} };
    useProjectStore.setState({ project });
    const [freeze] = items(freezeTrackEntries(transport, group, []));
    expect(freeze!.label).toBe("freeze group");
    expect(freeze!.disabled).toBe(true);
  });

  it("locks the children of a frozen group", () => {
    const { group, midi } = withGroup(true);
    expect(items(freezeTrackEntries(transport, midi, [midi.id])).map((e) => [e.label, e.disabled])).toEqual([["frozen with group “Band”", true]]);
    render(<FreezeHeaderStatus track={midi} transport={transport} />);
    expect(screen.getByRole("img", { name: `frozen with group ${group.name}` })).toBeTruthy();
    // The group itself shows the pressed snowflake.
    render(<FreezeHeaderStatus track={group} transport={transport} />);
    expect(screen.getByRole("button", { name: `unfreeze ${group.name}` })).toHaveAttribute("aria-pressed", "true");
    // Clips of a child cannot be bounced.
    const clip = Object.values(project.clips).find((c) => c.track === midi.id)!;
    expect(items(freezeClipEntries(transport, clip)).every((e) => e.disabled)).toBe(true);
  });
});
