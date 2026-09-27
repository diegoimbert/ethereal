/**
 * Mock of `MidiMap::*`: the document part (`Map`/`Edit`/`Unmap`, one mapping per source)
 * and the runtime part (`MockMidiLearn`: learn mode, `List`, simulated MIDI input driving
 * mapped targets). Owned by `midi-learn`.
 */

import type { Command, MidiControl, MidiMapCommand, MidiMapMode, MidiMapping, MidiMapTarget, ReplyValue } from "@/generated";
import { paramToPlain } from "@/features/devices/paramScale";
import { cmd } from "../../cmd";
import { builtinDescriptor } from "../builtinDevices";
import { fail, type ReducerContext } from "../documentReducer";
import type { MockHost } from "./host";

export function checkMapping(ctx: ReducerContext, m: MidiMapping): void {
  const { tx } = ctx;
  if (m.source.channel !== null && !(m.source.channel >= 0 && m.source.channel <= 15)) fail("InvalidArgument", "MIDI channel must be 0..=15");
  if (!(m.min >= 0 && m.min <= 1 && m.max >= 0 && m.max <= 1)) fail("InvalidArgument", "mapping range must be within 0..=1");
  const t = m.target;
  switch (t.type) {
    case "Param":
      switch (t.target.type) {
        case "TrackVolume":
        case "TrackPan":
          if (!tx.get("Track", t.target.track)) fail("NotFound", `track ${t.target.track}`);
          break;
        case "SendLevel":
          if (!tx.get("Send", t.target.send)) fail("NotFound", `send ${t.target.send}`);
          break;
        case "DeviceParam":
          if (!tx.get("Device", t.target.device)) fail("NotFound", `device ${t.target.device}`);
          break;
      }
      break;
    case "TrackMute":
    case "TrackSolo":
    case "TrackArm":
      if (!tx.get("Track", t.track)) fail("NotFound", `track ${t.track}`);
      break;
    case "Transport":
      break;
  }
}

/** Insert `m`, replacing a mapping with the same source (one undo step). */
export function putMapping(ctx: ReducerContext, m: MidiMapping): void {
  checkMapping(ctx, m);
  const same = JSON.stringify(m.source);
  for (const o of ctx.tx.all("MidiMapping")) {
    if (o.id !== m.id && JSON.stringify(o.source) === same) ctx.tx.remove("MidiMapping", o.id);
  }
  ctx.tx.upsert("MidiMapping", m);
}

/** Document part: `Map`, `Edit`, `Unmap`. */
export function midiMapCommand(ctx: ReducerContext, c: MidiMapCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "Map":
      putMapping(ctx, c.mapping);
      break;
    case "Edit": {
      const m = tx.get("MidiMapping", c.id) ?? fail("NotFound", `MIDI mapping ${c.id}`);
      const next = { ...m, min: c.min ?? m.min, max: c.max ?? m.max, mode: c.mode ?? m.mode };
      checkMapping(ctx, next);
      tx.upsert("MidiMapping", next);
      break;
    }
    case "Unmap":
      for (const id of c.ids) {
        if (!tx.get("MidiMapping", id)) fail("NotFound", `MIDI mapping ${id}`);
        tx.remove("MidiMapping", id);
      }
      break;
    default:
      fail("Internal", `not a document MIDI map command: ${c.type}`);
  }
}

const UNIT: ReplyValue = { type: "Unit" };

/**
 * Mode of a learned mapping, as the controller picks it: `Toggle` for notes and on/off
 * targets (mute, solo, arm), `Absolute` otherwise.
 */
export function learnMode(target: MidiMapTarget, control: MidiControl): MidiMapMode {
  const onOff = target.type === "TrackMute" || target.type === "TrackSolo" || target.type === "TrackArm";
  return onOff || control.type === "Note" ? { type: "Toggle" } : { type: "Absolute" };
}

/** Runtime part: learn mode, `List`, and simulated MIDI input. */
export class MockMidiLearn {
  private learnTarget: MidiMapTarget | null = null;

  constructor(private readonly host: MockHost) {}

  /** `MidiMap::Learn` / `MidiMap::List` (the other variants are document commands). */
  command(c: MidiMapCommand): ReplyValue {
    switch (c.type) {
      case "Learn":
        this.learnTarget = c.target;
        this.host.emit({ type: "MidiMap", event: { type: "LearnChanged", target: c.target } });
        return UNIT;
      case "List": {
        const mappings = Object.values(this.host.project().midi_mappings).sort((a, b) =>
          JSON.stringify(a.source).localeCompare(JSON.stringify(b.source)),
        );
        return { type: "MidiMappings", mappings };
      }
      default:
        return fail("Internal", `unreachable: ${c.type} is a document command`);
    }
  }

  /** One incoming MIDI short message (see `MockTransport.simulateMidiInput`). */
  input(port: string, data: [number, number, number]): void {
    const host = this.host;
    const [status, d1, d2] = data;
    const kind = status & 0xf0;
    const channel = status & 0x0f;
    let control: MidiControl;
    let value: number;
    if (kind === 0xb0) {
      control = { type: "Cc", number: d1 };
      value = d2 / 127;
    } else if (kind === 0x90 || kind === 0x80) {
      control = { type: "Note", key: d1 };
      value = kind === 0x90 ? d2 / 127 : 0;
    } else if (kind === 0xe0) {
      control = { type: "PitchBend" };
      value = ((d2 << 7) | d1) / 16383;
    } else {
      return;
    }
    const source = { port, channel, control };
    host.emit({ type: "MidiMap", event: { type: "Activity", source } });
    if (this.learnTarget) {
      // Like the controller: note-offs never complete a learn.
      if (control.type === "Note" && value === 0) return;
      const target = this.learnTarget;
      const mapping: MidiMapping = { id: host.newId(), source, target, min: 0, max: 1, mode: learnMode(target, control) };
      // The new mapping replaces every mapping of its target (and of its source, in `putMapping`).
      const key = JSON.stringify(target);
      const stale = Object.values(host.project().midi_mappings)
        .filter((m) => JSON.stringify(m.target) === key)
        .map((m) => m.id);
      host.applyDocument(
        [...(stale.length ? [cmd("MidiMap", { type: "Unmap", ids: stale })] : []), cmd("MidiMap", { type: "Map", mapping })],
        "MIDI Learn",
      );
      this.learnTarget = null;
      host.emit({ type: "MidiMap", event: { type: "Learned", mapping: mapping.id } });
      host.emit({ type: "MidiMap", event: { type: "LearnChanged", target: null } });
      return;
    }
    const project = host.project();
    const same = JSON.stringify(control);
    const m = Object.values(project.midi_mappings).find(
      (x) =>
        (x.source.port === null || x.source.port === port) &&
        (x.source.channel === null || x.source.channel === channel) &&
        JSON.stringify(x.source.control) === same,
    );
    if (!m) return;
    const v = m.min + value * (m.max - m.min);
    const faderDb = (n: number) => {
      const amp = n * n * n * Math.pow(10, 6 / 20);
      return amp <= 0 ? -144 : Math.max(-144, 20 * Math.log10(amp));
    };
    const apply = (c: Command) => {
      try {
        host.execute(c);
      } catch {
        // A stale mapping target is ignored, like on the real engine.
      }
    };
    const t = m.target;
    switch (t.type) {
      case "Param":
        switch (t.target.type) {
          case "TrackVolume":
            apply(cmd("Mixer", { type: "SetVolume", track: t.target.track, volume: faderDb(v) }));
            break;
          case "TrackPan":
            apply(cmd("Mixer", { type: "SetPan", track: t.target.track, pan: v * 2 - 1 }));
            break;
          case "SendLevel":
            apply(cmd("Mixer", { type: "SetSendLevel", send: t.target.send, level: faderDb(v) }));
            break;
          case "DeviceParam": {
            const d = project.devices[t.target.device];
            if (d?.kind.type !== "Builtin") return;
            const param = t.target.param;
            const info = builtinDescriptor(d.kind.device).params.find((p) => p.id === param);
            if (info) apply(cmd("Device", { type: "SetParam", device: d.id, param, value: paramToPlain(info, v) }));
            break;
          }
        }
        break;
      case "TrackMute":
        apply(cmd("Mixer", { type: "SetMute", track: t.track, mute: v >= 0.5 }));
        break;
      case "TrackSolo":
        apply(cmd("Mixer", { type: "SetSolo", track: t.track, solo: v >= 0.5, exclusive: false }));
        break;
      case "TrackArm":
        apply(cmd("Recording", { type: "Arm", track: t.track, armed: v >= 0.5, exclusive: false }));
        break;
      case "Transport": {
        if (v < 0.5) return;
        const s = project.settings;
        const byAction: Partial<Record<typeof t.action, Command>> = {
          Play: cmd("Transport", { type: "Play" }),
          Stop: cmd("Transport", { type: "Stop" }),
          TogglePlay: cmd("Transport", { type: "TogglePlay" }),
          ToggleLoop: cmd("Transport", { type: "SetLoopEnabled", enabled: !s.loop_enabled }),
          ToggleMetronome: cmd("Transport", { type: "SetMetronome", enabled: !s.metronome }),
          TapTempo: cmd("Transport", { type: "TapTempo" }),
        };
        const c = byAction[t.action];
        if (c) apply(c);
        break;
      }
    }
  }
}
