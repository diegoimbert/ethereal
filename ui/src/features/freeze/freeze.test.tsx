import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, Event, Project, ReplyValue, Track } from "@/generated";
import type { ContextMenuEntry, ContextMenuItem } from "@/kit";
import { useProjectStore } from "@/state";
import { createDemoProject, type EngineTransport } from "@/transport";
import { FreezeHeaderStatus } from "./FreezeHeaderStatus";
import { freezeClipEntries, freezeTrackEntries, withFreezeTrackEntries } from "./menus";
import { resetRenderJobs } from "./store";

/** A transport that records commands and lets the test push events / choose replies. */
class FakeTransport {
  readonly kind = "mock" as const;
  sent: Command[] = [];
  reply: (c: Command) => ReplyValue = (c) =>
    c.domain === "Freeze" && "job" in c.command && c.command.type !== "Cancel"
      ? { type: "RenderStarted", job: c.command.job }
      : { type: "Unit" };
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

afterEach(() => useProjectStore.setState({ project: null }));

describe("freeze menus", () => {
  it("offers Freeze before the destructive group, then Unfreeze / Flatten when frozen", () => {
    const t = trackOf("Midi");
    const menu = withFreezeTrackEntries(["separator", { label: "Delete Track", onSelect: () => {} }], transport, t, [t.id]);
    expect(labels(menu)).toEqual(["—", "Freeze Track", "—", "Delete Track"]);
    const frozen = { ...t, freeze: { media: "m", start: 0 } };
    expect(labels(freezeTrackEntries(transport, frozen, []))).toEqual(["Unfreeze Track", "Flatten Track"]);
    expect(freezeTrackEntries(transport, { ...t, kind: "Group" }, [])).toEqual([]);
  });

  it("offers bounce and consolidate on clips (in place for audio only)", () => {
    const midi = Object.values(project.clips).find((c) => c.content.type === "Midi")!;
    const audio = Object.values(project.clips).find((c) => c.content.type === "Audio")!;
    expect(labels(freezeClipEntries(transport, midi))).toEqual(["Bounce to New Track", "Consolidate"]);
    expect(labels(freezeClipEntries(transport, audio))).toEqual(["Bounce to New Track", "Bounce in Place", "Consolidate"]);
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
    expect(screen.getByRole("progressbar", { name: `Freezing ${t.name}` })).toHaveAttribute("aria-valuenow", "40");
    // While it runs, the menu offers Cancel.
    expect(labels(freezeTrackEntries(transport, t, []))).toEqual(["Cancel Freeze"]);
    act(() => fake.emit({ type: "Freeze", event: { type: "Done", job } }));
    expect(screen.queryByRole("progressbar")).toBeNull();
    rerender(<FreezeHeaderStatus track={{ ...t, freeze: { media: "m", start: 0 } }} transport={transport} />);
    fireEvent.click(screen.getByRole("button", { name: `Unfreeze ${t.name}` }));
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
    expect(screen.getByRole("progressbar", { name: `Freezing ${audio.name}` })).not.toHaveAttribute("aria-valuenow");
    const job = (fake.sent[0]!.command as { job: string }).job;
    act(() => fake.emit({ type: "Freeze", event: { type: "Failed", job, message: "disk full" } }));
    await flush();
    expect(fake.sent).toHaveLength(2);
    const warning = screen.getByRole("button", { name: "Render failed: disk full" });
    fireEvent.click(warning);
    expect(screen.queryByRole("button", { name: /Render failed/ })).toBeNull();
  });

  it("cancels a running job", async () => {
    const t = trackOf("Midi");
    render(<FreezeHeaderStatus track={t} transport={transport} />);
    items(freezeTrackEntries(transport, t, []))[0]!.onSelect();
    await flush();
    const job = (fake.sent[0]!.command as { job: string }).job;
    fireEvent.click(screen.getByRole("button", { name: `Cancel freeze of ${t.name}` }));
    await flush();
    expect(fake.sent.at(-1)).toEqual({ domain: "Freeze", command: { type: "Cancel", job } });
    act(() => fake.emit({ type: "Freeze", event: { type: "Cancelled", job } }));
    expect(screen.queryByRole("progressbar")).toBeNull();
  });
});
