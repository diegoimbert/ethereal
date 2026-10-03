import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AudioToMidiCommand, Clip, Command, Event, Project, ReplyValue } from "@/generated";
import type { ContextMenuEntry, ContextMenuItem } from "@/kit";
import { useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { CommandFailedError, createDemoProject, type EngineTransport } from "@/transport";
import { AudioToMidiClipStatus } from "./ClipStatus";
import { audioToMidiClipEntries, withAudioToMidiEntries } from "./menus";
import { DEFAULT_OPTIONS, resetAudioToMidi, useAudioToMidi } from "./store";

/** A transport that records commands and lets the test push events / choose replies. */
class FakeTransport {
  readonly kind = "mock" as const;
  sent: Command[] = [];
  reply: (c: Command) => ReplyValue = () => ({ type: "Unit" });
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
let audio: Clip;

const starts = () =>
  fake.sent
    .filter((c): c is Extract<Command, { domain: "AudioToMidi" }> => c.domain === "AudioToMidi")
    .map((c) => c.command)
    .filter((c): c is Extract<AudioToMidiCommand, { type: "Start" }> => c.type === "Start");

const a2m = (event: Extract<Event, { type: "AudioToMidi" }>["event"]) => act(() => fake.emit({ type: "AudioToMidi", event }));

beforeEach(() => {
  project = createDemoProject();
  useProjectStore.setState({ project });
  resetAudioToMidi();
  fake = new FakeTransport();
  transport = fake as unknown as EngineTransport;
  audio = Object.values(project.clips).find((c) => c.content.type === "Audio")!;
});

afterEach(() => useProjectStore.setState({ project: null }));

describe("audio to MIDI menu", () => {
  it("offers Convert to MIDI… on audio clips, before the destructive group", () => {
    const midi = Object.values(project.clips).find((c) => c.content.type === "Midi")!;
    expect(audioToMidiClipEntries(transport, midi)).toEqual([]);
    const menu = withAudioToMidiEntries(["separator", { label: "Delete", onSelect: () => {} }], transport, audio);
    expect(labels(menu)).toEqual(["—", "Convert to MIDI…", "—", "Delete"]);
    items(audioToMidiClipEntries(transport, audio))[0]!.onSelect();
    expect(useAudioToMidi.getState().dialog).toBe(audio.id);
  });

  it("offers Stop on the converting clip and disables others while busy", () => {
    useAudioToMidi.setState({ job: { job: "j", clip: audio.id, mode: "Melody", progress: 0.2, track: "t", newClip: "c" } });
    const [stop] = items(audioToMidiClipEntries(transport, audio));
    expect(stop!.label).toBe("Stop Converting to MIDI");
    stop!.onSelect();
    expect(fake.sent.at(-1)).toEqual({ domain: "AudioToMidi", command: { type: "Cancel", job: "j" } });
    const other = { ...audio, id: "other" };
    useProjectStore.setState({ project: { ...project, clips: { ...project.clips, other } } });
    expect(items(audioToMidiClipEntries(transport, other))[0]).toMatchObject({ label: "Convert to MIDI…", disabled: true });
  });
});

describe("convert dialog", () => {
  it("converts with the chosen settings, shows progress, then closes and selects the new clip", async () => {
    render(<AudioToMidiClipStatus clip={audio} transport={transport} />);
    act(() => useAudioToMidi.setState({ dialog: audio.id }));
    expect(await screen.findByTestId("audio-to-midi-dialog")).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: "Drums" }));
    expect(screen.getByText(/Kick, snare and hi-hat/)).toBeTruthy();
    fireEvent.click(screen.getByRole("switch"));
    fireEvent.click(screen.getByRole("button", { name: "Convert" }));
    await flush();
    const [start] = starts();
    expect(start).toMatchObject({ clip: audio.id, mode: "Drums", options: DEFAULT_OPTIONS, instrument: null });

    a2m({ type: "Progress", job: start!.job, progress: 0.4 });
    expect(screen.getAllByRole("progressbar").map((p) => p.getAttribute("aria-valuenow"))).toContain("40");
    expect(screen.getByRole("button", { name: /Stop converting/ })).toBeTruthy();
    // Settings are locked while converting.
    expect((screen.getByRole("radio", { name: "Melody" }) as HTMLButtonElement).disabled).toBe(true);

    a2m({ type: "Done", job: start!.job, track: start!.track, clip: start!.new_clip, notes: 12 });
    expect(useAudioToMidi.getState()).toMatchObject({ job: null, dialog: null });
    expect([...itemSelection.getState().selected.clip]).toEqual([start!.new_clip]);
    expect(screen.queryAllByRole("progressbar")).toHaveLength(0);
  });

  it("stops from the clip; a failure shows on the clip and in the dialog", async () => {
    render(<AudioToMidiClipStatus clip={audio} transport={transport} />);
    act(() => useAudioToMidi.setState({ dialog: audio.id }));
    fireEvent.click(await screen.findByRole("button", { name: "Convert" }));
    await flush();
    const job = starts()[0]!.job;
    fireEvent.click(screen.getByRole("button", { name: "Hide" }));
    expect(useAudioToMidi.getState().dialog).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Stop converting/ }));
    await flush();
    expect(fake.sent.at(-1)).toEqual({ domain: "AudioToMidi", command: { type: "Cancel", job } });
    a2m({ type: "Cancelled", job });
    expect(useAudioToMidi.getState().job).toBeNull();

    // A failed start (reply) and a failed job (event) both end up on the clip.
    fake.reply = () => {
      throw new CommandFailedError({ code: "InvalidState", message: "busy" });
    };
    act(() => useAudioToMidi.setState({ dialog: audio.id }));
    fireEvent.click(await screen.findByRole("button", { name: "Convert" }));
    await flush();
    expect(screen.getByRole("alert").textContent).toContain("busy");
    fake.reply = () => ({ type: "Unit" });
    fireEvent.click(screen.getByRole("button", { name: "Convert" }));
    await flush();
    a2m({ type: "Failed", job: starts().at(-1)!.job, message: "could not be decoded" });
    act(() => useAudioToMidi.setState({ dialog: null }));
    const warn = screen.getByRole("button", { name: /Convert to MIDI failed: could not be decoded/ });
    fireEvent.click(warn);
    expect(useAudioToMidi.getState().dialog).toBe(audio.id);
  });
});
