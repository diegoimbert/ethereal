import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Command, Note } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { GrooveControls } from "./GrooveControls";
import { GroovePanel } from "./GroovePanel";
import {
  grooveQuantizeCommand,
  humanizeCommand,
  HUMANIZE_TIMING_UNIT,
  setSwingCommand,
  swingGridOf,
  useGrooveSettings,
} from "./grooveCommands";

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();
const CLIP = "01GROOVETESTCLIP0000000000";
const A = "01GROOVENOTEA0000000000000";
const B = "01GROOVENOTEB0000000000000";
const note = (id: string): Note => store().project!.notes[id]!;

afterEach(() => {
  vi.restoreAllMocks();
  mock?.dispose();
  mock = undefined;
  store().reset();
  useGrooveSettings.getState().reset();
});

async function send(command: Command) {
  await act(async () => {
    await mock!.send(command);
  });
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function setup(ui: "controls" | "panel", selected: string[] = []) {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      {ui === "controls" ? <GrooveControls clip={CLIP} selected={selected} rollStep={1} /> : <GroovePanel />}
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
  await send(cmd("Clip", { type: "CreateMidi", id: CLIP, track: track.id, start: 0, length: 8, name: "G" }));
  await send(
    cmd("Note", {
      type: "Add",
      clip: CLIP,
      notes: [
        { id: A, pitch: 60, velocity: 0.5, start: 0.1, duration: 0.25 },
        { id: B, pitch: 62, velocity: 0.5, start: 0.45, duration: 0.25 },
      ],
    }),
  );
}

function setPercent(label: string, value: number) {
  const input = screen.getByRole("spinbutton", { name: label });
  fireEvent.change(input, { target: { value: String(value) } });
  fireEvent.keyDown(input, { key: "Enter" });
}

describe("groove commands", () => {
  it("quantize carries grid, strength, swing and ends", () => {
    const c = grooveQuantizeCommand(CLIP, [A], { grid: "1/8", strength: 0.5, swing: 1, ends: true }, 1);
    expect(c).toEqual(
      cmd("Note", { type: "Quantize", clip: CLIP, notes: [A], grid: 0.5, strength: 0.5, ends: true, swing: 1 }),
    );
    // "Roll grid" follows the piano roll's step; no selection = whole clip.
    const all = grooveQuantizeCommand(CLIP, [], { grid: "roll", strength: 1, swing: 0, ends: false }, 0.25);
    expect(all).toMatchObject({ command: { notes: null, grid: 0.25 } });
  });

  it("humanize scales timing to a sixteenth and keeps the seed as u32", () => {
    expect(humanizeCommand(CLIP, [], { timing: 0.5, velocity: 0.2 }, -1)).toEqual(
      cmd("Groove", {
        type: "Humanize",
        clip: CLIP,
        notes: null,
        timing: 0.5 * HUMANIZE_TIMING_UNIT,
        velocity: 0.2,
        seed: 0xffffffff,
      }),
    );
  });

  it("set swing clamps the amount; grid choices round-trip", () => {
    expect(setSwingCommand(2, 0.5)).toEqual(cmd("Groove", { type: "SetSwing", amount: 1, grid: 0.5 }));
    expect(swingGridOf(0.5)).toBe("1/8");
    expect(swingGridOf(0.25)).toBe("1/16");
  });
});

describe("GrooveControls (piano roll)", () => {
  it("quantizes with strength 50% halfway and swing on off-beats", async () => {
    await setup("controls");
    fireEvent.click(screen.getByRole("button", { name: "Quantize…" }));
    fireEvent.change(screen.getByLabelText("Quantize grid"), { target: { value: "1/8" } });
    setPercent("Strength", 50);
    setPercent("Swing", 100);
    fireEvent.click(screen.getByRole("button", { name: "Quantize all notes" }));
    await flush();
    // A: 0.1 → target 0 → halfway 0.05. B: 0.45 → target 0.5 + 0.5/3 (odd eighth) → halfway.
    expect(note(A).start).toBeCloseTo(0.05, 9);
    expect(note(B).start).toBeCloseTo(0.45 + (0.5 + 0.5 / 3 - 0.45) / 2, 9);
    // Settings are remembered for the next open.
    expect(useGrooveSettings.getState().quantize).toMatchObject({ grid: "1/8", strength: 0.5, swing: 1 });
  });

  it("humanizes the selection deterministically from the seed (one undo step)", async () => {
    await setup("controls", [A]);
    vi.spyOn(Math, "random").mockReturnValue(0.25);
    fireEvent.click(screen.getByRole("button", { name: "Humanize…" }));
    setPercent("Timing (±1/16)", 40);
    setPercent("Velocity", 20);
    const spy = vi.spyOn(mock!, "send");
    fireEvent.click(screen.getByRole("button", { name: "Humanize 1 selected" }));
    await flush();
    const sent = spy.mock.calls[0]![0];
    expect(sent).toEqual(
      cmd("Groove", { type: "Humanize", clip: CLIP, notes: [A], timing: 0.1, velocity: 0.2, seed: 0x40000000 }),
    );
    const first = { ...note(A) };
    expect(first.start).not.toBe(0.1);
    expect(note(B).start).toBe(0.45);
    // Undo, then the same seed gives the same result.
    await send(cmd("Edit", { type: "Undo" }));
    expect(note(A).start).toBe(0.1);
    await send(sent);
    expect(note(A).start).toBe(first.start);
    expect(note(A).velocity).toBe(first.velocity);
  });
});

describe("GroovePanel", () => {
  it("sets the project swing amount and grid", async () => {
    await setup("panel");
    setPercent("Project swing", 60);
    await flush();
    expect(store().project!.settings.swing).toBeCloseTo(0.6, 6);
    fireEvent.change(screen.getByLabelText("Swing grid"), { target: { value: "1/8" } });
    await flush();
    expect(store().project!.settings.swing_grid).toBe(0.5);
    expect(store().project!.settings.swing).toBeCloseTo(0.6, 6);
  });
});
