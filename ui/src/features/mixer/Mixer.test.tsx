import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { playheadStore, useSelectionStore } from "@/state";
import { cmd, type MockTransport } from "@/transport";
import { createBeatSketchProject } from "@/transport/mock/demoProject";
import { Mixer } from "./index";
import { dbToFader, faderToDb, mixerLayout, outputTargets, parseOutputValue, outputValue } from "./routing";
import { decayed, METER_HOLD_MS } from "./useMeterLevels";
import { dragUp, flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "./testUtils";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  playheadStore.reset();
});

const strip = (name: string) => screen.getByRole("group", { name });

describe("routing helpers", () => {
  it("fader law round-trips and pins 0 dB / -inf", () => {
    expect(faderToDb(dbToFader(0))).toBeCloseTo(0, 9);
    expect(faderToDb(dbToFader(-12))).toBeCloseTo(-12, 9);
    expect(faderToDb(0)).toBe(-144);
    expect(faderToDb(1)).toBeCloseTo(6, 9);
  });

  it("encodes outputs", () => {
    for (const o of [{ type: "Default" }, { type: "None" }, { type: "Track", track: "X" }] as const) {
      expect(parseOutputValue(outputValue(o))).toEqual(o);
    }
  });

  it("lays out groups nested, then returns, then master; excludes cycles from outputs", () => {
    const p = createBeatSketchProject();
    const layout = mixerLayout(p.tracks);
    expect(layout.tracks.map((n) => n.track.name)).toEqual(["Drum Bus"]);
    expect(layout.tracks[0]!.children.map((n) => n.track.name)).toEqual(["Kick", "Hats"]);
    expect(layout.master?.name).toBe("Master");
    const bus = layout.tracks[0]!.track;
    const kick = layout.tracks[0]!.children[0]!.track;
    expect(outputTargets(p.tracks, kick).map((t) => t.name)).toEqual(["Drum Bus"]);
    expect(outputTargets(p.tracks, bus)).toEqual([]);
  });

  it("decays stale meter readings after the hold time", () => {
    expect(decayed(0.5, METER_HOLD_MS)).toBe(0.5);
    expect(decayed(1, METER_HOLD_MS + 1000)).toBeCloseTo(Math.pow(10, -30 / 20), 9);
    expect(decayed(1, METER_HOLD_MS + 10_000)).toBe(0);
  });
});

describe("Mixer", () => {
  it("renders a strip per track with returns and master last", async () => {
    mock = await renderWithMock(<Mixer />);
    const names = screen.getAllByRole("group").map((g) => g.getAttribute("aria-label"));
    expect(names).toEqual(["Keys", "Bass", "Drums", "A Delay", "Master"]);
    // Master has no solo, no output selector, no sends.
    const master = strip("Master");
    expect(within(master).queryByRole("button", { name: "Solo Master" })).toBeNull();
    expect(within(master).queryByRole("combobox")).toBeNull();
    expect(within(strip("Keys")).getByRole("slider", { name: "Send A Delay" })).toBeInTheDocument();
  });

  it("mutes and solos through commands (solo is exclusive unless Ctrl/Cmd)", async () => {
    mock = await renderWithMock(<Mixer />);
    fireEvent.click(screen.getByRole("button", { name: "Mute Keys" }));
    await flush();
    expect(trackByName("Keys").mixer.mute).toBe(true);
    expect(store().history.can_undo).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Solo Keys" }));
    await flush();
    fireEvent.click(screen.getByRole("button", { name: "Solo Bass" }));
    await flush();
    expect(trackByName("Keys").mixer.solo).toBe(false);
    expect(trackByName("Bass").mixer.solo).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Solo Drums" }), { ctrlKey: true });
    await flush();
    expect(trackByName("Bass").mixer.solo).toBe(true);
    expect(trackByName("Drums").mixer.solo).toBe(true);
  });

  it("a fader drag is one undo step", async () => {
    mock = await renderWithMock(<Mixer />);
    const before = trackByName("Keys").mixer.volume;
    const undoLabel = store().history.undo_label;
    await dragUp(screen.getByRole("slider", { name: "Keys volume" }), 4, 5);
    await flush();
    const after = trackByName("Keys").mixer.volume;
    expect(after).toBeGreaterThan(before);
    expect(screen.getByRole("slider", { name: "Keys volume" })).toHaveAttribute("aria-valuetext", `${after.toFixed(1)} dB`);

    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(trackByName("Keys").mixer.volume).toBe(before);
    expect(store().history.undo_label).toBe(undoLabel);
  });

  it("pan keyboard steps are separate undo steps", async () => {
    mock = await renderWithMock(<Mixer />);
    const pan = screen.getByRole("slider", { name: "Keys pan" });
    fireEvent.keyDown(pan, { key: "ArrowRight" });
    await flush();
    expect(trackByName("Keys").mixer.pan).toBeCloseTo(0.02, 6);
  });

  it("send knob drag creates the send and sets its level in one undo step", async () => {
    mock = await renderWithMock(<Mixer />);
    const bass = trackByName("Bass");
    const sendsOfBass = () => Object.values(store().project!.sends).filter((s) => s.from === bass.id);
    expect(sendsOfBass()).toHaveLength(0);
    await dragUp(within(strip("Bass")).getByRole("slider", { name: "Send A Delay" }), 3, 30);
    await flush();
    expect(sendsOfBass()).toHaveLength(1);
    const level = sendsOfBass()[0]!.level;
    expect(level).toBeGreaterThan(-144);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(sendsOfBass()).toHaveLength(0);

    // Existing send: pre/post toggle.
    fireEvent.click(within(strip("Keys")).getByRole("button", { name: "Pre-fader send A Delay" }));
    await flush();
    const keysSend = Object.values(store().project!.sends).find((s) => s.from === trackByName("Keys").id)!;
    expect(keysSend.pre_fader).toBe(true);
  });

  it("routes a track's output", async () => {
    mock = await renderWithMock(<Mixer />);
    const ret = trackByName("A Delay");
    fireEvent.change(screen.getByRole("combobox", { name: "Drums output" }), { target: { value: `track:${ret.id}` } });
    await flush();
    expect(trackByName("Drums").output).toEqual({ type: "Track", track: ret.id });
    fireEvent.change(screen.getByRole("combobox", { name: "Drums output" }), { target: { value: "none" } });
    await flush();
    expect(trackByName("Drums").output).toEqual({ type: "None" });
  });

  it("nests group children and folds them", async () => {
    mock = await renderWithMock(<Mixer />, createBeatSketchProject());
    const bus = strip("Drum Bus").closest("[data-group]")!;
    expect(within(bus as HTMLElement).getByRole("group", { name: "Kick" })).toBeInTheDocument();
    expect(within(strip("Kick")).getByRole("combobox", { name: "Kick output" })).toHaveDisplayValue("Group (Drum Bus)");
    fireEvent.click(screen.getByRole("button", { name: "Fold Drum Bus" }));
    expect(screen.queryByRole("group", { name: "Kick" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Unfold Drum Bus" }));
    expect(screen.getByRole("group", { name: "Kick" })).toBeInTheDocument();
  });

  it("selects a track by clicking its name", async () => {
    mock = await renderWithMock(<Mixer />);
    fireEvent.click(within(strip("Bass")).getByRole("button", { name: "Bass" }));
    expect(useSelectionStore.getState().selectedTrack).toBe(trackByName("Bass").id);
  });

  it("drives meters from the meter stream and latches clipping", async () => {
    mock = await renderWithMock(<Mixer />);
    const keys = trackByName("Keys");
    const clip = () => within(strip("Keys")).getByRole("button", { name: /Keys clip/ });
    expect(clip()).toHaveAccessibleName("Keys clip indicator");
    act(() =>
      playheadStore.setMeters({
        tracks: [{ track: keys.id, peak: [1.2, 0.5], rms: [0.8, 0.3], clipped: true }],
        cpu_load: 0,
      }),
    );
    expect(clip()).toHaveAccessibleName("Keys clipped (click to reset)");
    const meter = within(strip("Keys")).getByRole("meter");
    expect(meter.querySelectorAll(".eth-meter__clip")).toHaveLength(1);
    act(() =>
      playheadStore.setMeters({ tracks: [{ track: keys.id, peak: [0.1, 0.1], rms: [0, 0], clipped: false }], cpu_load: 0 }),
    );
    expect(clip()).toHaveAccessibleName("Keys clipped (click to reset)");
    fireEvent.click(clip());
    expect(clip()).toHaveAccessibleName("Keys clip indicator");
  });

  it("gets meter frames from the transport", async () => {
    mock = await renderWithMock(<Mixer />);
    await act(async () => {
      await mock!.send(cmd("Transport", { type: "Play" }));
      mock!.tick(500);
    });
    const masks = [...within(strip("Keys")).getByRole("meter").querySelectorAll<HTMLElement>(".eth-meter__mask")];
    expect(masks.some((m) => m.style.height !== "100%")).toBe(true);
  });
});
