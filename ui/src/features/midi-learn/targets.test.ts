import { describe, expect, it } from "vitest";
import type { MidiMapTarget, Project } from "@/generated";
import { reduceMidiEvent } from "./store";
import {
  describeSource,
  describeTarget,
  MAPPABLE_SELECTOR,
  midiTarget,
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

  it("resolves controls from their data-midi-target attribute only", () => {
    const p = project();
    const t = (target: MidiMapTarget) => JSON.stringify(target);
    const root = dom(`
      <div class="eth-strip" data-track="t1">
        <div data-midi-target='${t({ type: "Param", target: { type: "SendLevel", send: "s1" } })}'><svg><circle class="in-send"/></svg></div>
        <button class="pre">Pre</button>
        <div class="eth-knob eth-strip__pan" data-midi-target='${t({ type: "Param", target: { type: "TrackPan", track: "t1" } })}'><svg><circle class="in-pan"/></svg></div>
        <div data-midi-target='${t({ type: "Param", target: { type: "TrackVolume", track: "t1" } })}'><div class="eth-fader__thumb"></div></div>
        <button class="mute" data-midi-target='${t({ type: "TrackMute", track: "t1" })}'>M</button>
        <button class="solo" data-midi-target='${t({ type: "TrackSolo", track: "t1" })}'>S</button>
        <button class="arm" data-midi-target='${t({ type: "TrackArm", track: "t1" })}'>A</button>
        <button class="eth-strip__mute eth-strip__name">Bass</button>
      </div>
      <div data-midi-target='${t({ type: "Param", target: { type: "DeviceParam", device: "d1", param: 4 } })}'><div class="inner"></div></div>
      <div data-midi-target='${t({ type: "Param", target: { type: "DeviceParam", device: "gone", param: 4 } })}'><span class="ghost"></span></div>
      <div data-midi-target='${t({ type: "TrackArm", track: "gone" })}'><span class="ghost-track"></span></div>
      <div data-midi-target='not json'><span class="broken"></span></div>
      <div class="eth-tb">
        <button class="play" data-midi-target='${t({ type: "Transport", action: "TogglePlay" })}'></button>
        <button class="eth-tb__record" aria-label="Record"></button>
      </div>
    `);
    const at = (sel: string) => resolveControl(root.querySelector(sel)!, p)?.target ?? null;
    expect(at(".in-send")).toEqual({ type: "Param", target: { type: "SendLevel", send: "s1" } });
    expect(at(".pre")).toBeNull();
    expect(at(".in-pan")).toEqual({ type: "Param", target: { type: "TrackPan", track: "t1" } });
    expect(at(".eth-fader__thumb")).toEqual({ type: "Param", target: { type: "TrackVolume", track: "t1" } });
    expect(at(".mute")).toEqual({ type: "TrackMute", track: "t1" });
    expect(at(".solo")).toEqual({ type: "TrackSolo", track: "t1" });
    expect(at(".arm")).toEqual({ type: "TrackArm", track: "t1" });
    // Class names alone never make a control mappable.
    expect(at(".eth-strip__name")).toBeNull();
    expect(at(".eth-tb__record")).toBeNull();
    expect(at(".inner")).toEqual({ type: "Param", target: { type: "DeviceParam", device: "d1", param: 4 } });
    expect(at(".ghost")).toBeNull();
    expect(at(".ghost-track")).toBeNull();
    expect(at(".broken")).toBeNull();
    expect(at(".play")).toEqual({ type: "Transport", action: "TogglePlay" });
    expect(resolveControl(root.querySelector(".in-pan")!, null)).toBeNull();

    // Every mappable element is found by the selector and resolves to itself.
    const marked = [...root.querySelectorAll(MAPPABLE_SELECTOR)].filter((el) => resolveControl(el, p)?.element === el);
    expect(marked.length).toBe(8);
  });

  it("midiTarget() builds the attribute that resolveControl reads", () => {
    const target: MidiMapTarget = { type: "Param", target: { type: "SendLevel", send: "s1" } };
    const el = document.createElement("div");
    for (const [k, v] of Object.entries(midiTarget(target))) el.setAttribute(k, v);
    expect(el.getAttribute("data-midi-target")).toBe(JSON.stringify(target));
    expect(resolveControl(el, project())).toEqual({ element: el, target });
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
