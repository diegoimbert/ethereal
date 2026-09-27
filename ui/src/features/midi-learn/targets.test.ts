import { describe, expect, it } from "vitest";
import type { MidiMapTarget, Project } from "@/generated";
import { reduceMidiEvent } from "./store";
import {
  describeSource,
  describeTarget,
  MAPPABLE_SELECTOR,
  MODE_OPTIONS,
  modeValue,
  noteName,
  parseMode,
  resolveControl,
  sameTarget,
  sourceMatches,
  targetKey,
} from "./targets";

function project(): Project {
  return {
    tracks: {
      t1: { id: "t1", name: "Bass" },
      t2: { id: "t2", name: "Reverb" },
    },
    sends: { s1: { id: "s1", from: "t1", to: "t2" } },
    devices: { d1: { id: "d1", name: "Delay" } },
    midi_mappings: {},
  } as unknown as Project;
}

function dom(html: string): HTMLElement {
  const root = document.createElement("div");
  root.innerHTML = html;
  return root;
}

describe("targets", () => {
  it("keys and describes targets", () => {
    const p = project();
    const vol: MidiMapTarget = { type: "Param", target: { type: "TrackVolume", track: "t1" } };
    expect(targetKey(vol)).toBe("vol:t1");
    expect(sameTarget(vol, { type: "Param", target: { type: "TrackVolume", track: "t1" } })).toBe(true);
    expect(sameTarget(vol, { type: "TrackMute", track: "t1" })).toBe(false);
    expect(describeTarget(vol, p)).toBe("Bass · Volume");
    expect(describeTarget({ type: "Param", target: { type: "SendLevel", send: "s1" } }, p)).toBe("Bass · Send Reverb");
    expect(describeTarget({ type: "Param", target: { type: "DeviceParam", device: "d1", param: 2 } }, p, { d1: { 2: "Feedback" } })).toBe(
      "Delay · Feedback",
    );
    expect(describeTarget({ type: "Param", target: { type: "DeviceParam", device: "d1", param: 3 } }, p)).toBe("Delay · Param 3");
    expect(describeTarget({ type: "Transport", action: "ToggleLoop" }, p)).toBe("Transport · Loop");
    expect(describeTarget({ type: "TrackArm", track: "gone" }, p)).toBe("Missing track · Arm");
  });

  it("describes and matches sources", () => {
    expect(noteName(60)).toBe("C3");
    expect(describeSource({ port: "Knobs", channel: 0, control: { type: "Cc", number: 21 } })).toBe("CC 21 · Ch 1 · Knobs");
    expect(describeSource({ port: null, channel: null, control: { type: "Note", key: 61 } })).toBe("Note C#3 · Any ch · Any port");
    const seen = { port: "Knobs", channel: 3, control: { type: "Cc", number: 7 } } as const;
    expect(sourceMatches({ port: null, channel: null, control: { type: "Cc", number: 7 } }, seen)).toBe(true);
    expect(sourceMatches({ port: "Other", channel: null, control: { type: "Cc", number: 7 } }, seen)).toBe(false);
    expect(sourceMatches({ port: "Knobs", channel: 3, control: { type: "Cc", number: 8 } }, seen)).toBe(false);
  });

  it("round-trips modes", () => {
    for (const o of MODE_OPTIONS) expect(modeValue(parseMode(o.value))).toBe(o.value);
    expect(parseMode("Relative:BinaryOffset")).toEqual({ type: "Relative", encoding: "BinaryOffset" });
  });

  it("resolves mixer, device and transport controls from the DOM", () => {
    const p = project();
    const root = dom(`
      <div class="eth-strip" data-track="t1">
        <div class="eth-strip__send" data-send-to="t2"><div class="eth-knob"><svg><circle class="in-send"/></svg></div><button class="pre">Pre</button></div>
        <div class="eth-knob eth-strip__pan"><svg><circle class="in-pan"/></svg></div>
        <div class="eth-strip__fader-row"><div class="eth-fader"><div class="eth-fader__thumb"></div></div></div>
        <button class="eth-strip__mute">M</button>
        <button class="eth-strip__solo">S</button>
        <button class="eth-strip__name">Bass</button>
      </div>
      <section data-device="d1"><div class="eth-param" data-param="4"><div class="eth-knob inner"></div></div></section>
      <section data-device="gone"><div class="eth-param" data-param="4"><span class="ghost"></span></div></section>
      <div class="eth-tb">
        <button class="eth-tb__play" aria-label="Play"></button>
        <button aria-label="Stop"></button>
        <button class="eth-tb__record" aria-label="Record"></button>
        <button aria-label="Loop"></button>
        <button aria-label="Metronome"></button>
        <button title="Tap tempo">TAP</button>
        <button aria-label="Undo"></button>
      </div>
      <div data-midi-target='{"type":"TrackArm","track":"t1"}'><span class="explicit"></span></div>
    `);
    const at = (sel: string) => resolveControl(root.querySelector(sel)!, p)?.target ?? null;
    expect(at(".in-send")).toEqual({ type: "Param", target: { type: "SendLevel", send: "s1" } });
    expect(at(".pre")).toBeNull();
    expect(at(".in-pan")).toEqual({ type: "Param", target: { type: "TrackPan", track: "t1" } });
    expect(at(".eth-fader__thumb")).toEqual({ type: "Param", target: { type: "TrackVolume", track: "t1" } });
    expect(at(".eth-strip__mute")).toEqual({ type: "TrackMute", track: "t1" });
    expect(at(".eth-strip__solo")).toEqual({ type: "TrackSolo", track: "t1" });
    expect(at(".eth-strip__name")).toBeNull();
    expect(at(".inner")).toEqual({ type: "Param", target: { type: "DeviceParam", device: "d1", param: 4 } });
    expect(at(".ghost")).toBeNull();
    expect(at(".eth-tb__play")).toEqual({ type: "Transport", action: "TogglePlay" });
    expect(at('[aria-label="Stop"]')).toEqual({ type: "Transport", action: "Stop" });
    expect(at(".eth-tb__record")).toEqual({ type: "Transport", action: "ToggleRecord" });
    expect(at('[aria-label="Loop"]')).toEqual({ type: "Transport", action: "ToggleLoop" });
    expect(at('[aria-label="Metronome"]')).toEqual({ type: "Transport", action: "ToggleMetronome" });
    expect(at('[title="Tap tempo"]')).toEqual({ type: "Transport", action: "TapTempo" });
    expect(at('[aria-label="Undo"]')).toBeNull();
    expect(at(".explicit")).toEqual({ type: "TrackArm", track: "t1" });
    expect(resolveControl(root.querySelector(".in-pan")!, null)).toBeNull();

    // Every mappable element is found by the selector and resolves to itself.
    const marked = [...root.querySelectorAll(MAPPABLE_SELECTOR)].filter((el) => resolveControl(el, p)?.element === el);
    expect(marked.length).toBe(13);
  });

  it("reduces engine events", () => {
    const s = { learning: null, learned: null, activity: null, activitySeq: 0 };
    const target: MidiMapTarget = { type: "TrackMute", track: "t1" };
    expect(reduceMidiEvent(s, { type: "LearnChanged", target })).toEqual({ learning: target });
    expect(reduceMidiEvent(s, { type: "Learned", mapping: "m1" })).toEqual({ learned: "m1" });
    const source = { port: "K", channel: 0, control: { type: "PitchBend" } } as const;
    expect(reduceMidiEvent(s, { type: "Activity", source })).toEqual({ activity: source, activitySeq: 1 });
  });
});
