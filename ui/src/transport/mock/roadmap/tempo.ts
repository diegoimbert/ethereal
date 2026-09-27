/**
 * Mock of `Tempo::*` (tempo map CRUD + metronome settings). Owned by `tempo-metronome`.
 * The points at beat 0 can't be removed or moved away from 0.
 */

import type { TempoCommand, TimeSignature } from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";
import { fail, type ReducerContext } from "../documentReducer";
import { clamp } from "./shared";

function validSignature(sig: TimeSignature): boolean {
  return Number.isInteger(sig.numerator) && sig.numerator >= 1 && sig.numerator <= 99 && [1, 2, 4, 8, 16, 32].includes(sig.denominator);
}

export function tempoCommand(ctx: ReducerContext, c: TempoCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "AddTempoPoint":
      if (tx.get("TempoPoint", c.id)) return;
      if (!(c.time >= 0)) fail("InvalidArgument", "tempo point time must be >= 0");
      tx.upsert("TempoPoint", { id: c.id, time: c.time, bpm: clamp(c.bpm, 20, 999), curve: c.curve });
      break;
    case "EditTempoPoint": {
      const p = tx.get("TempoPoint", c.id) ?? fail("NotFound", `tempo point ${c.id}`);
      if (c.time !== null && p.time < BEATS_EPSILON && c.time > BEATS_EPSILON) fail("InvalidArgument", "the tempo point at beat 0 cannot move");
      tx.upsert("TempoPoint", {
        ...p,
        time: c.time !== null ? Math.max(0, c.time) : p.time,
        bpm: c.bpm !== null ? clamp(c.bpm, 20, 999) : p.bpm,
        curve: c.curve ?? p.curve,
      });
      break;
    }
    case "RemoveTempoPoints":
      for (const id of c.ids) {
        const p = tx.get("TempoPoint", id) ?? fail("NotFound", `tempo point ${id}`);
        if (p.time < BEATS_EPSILON) fail("InvalidArgument", "the tempo point at beat 0 cannot be removed");
        tx.remove("TempoPoint", id);
      }
      break;
    case "AddTimeSignature":
      if (tx.get("TimeSignature", c.id)) return;
      if (!validSignature(c.signature)) fail("InvalidArgument", "invalid time signature");
      if (!(c.time >= 0)) fail("InvalidArgument", "time signature time must be >= 0");
      tx.upsert("TimeSignature", { id: c.id, time: c.time, signature: c.signature });
      break;
    case "EditTimeSignature": {
      const p = tx.get("TimeSignature", c.id) ?? fail("NotFound", `time signature ${c.id}`);
      if (c.signature !== null && !validSignature(c.signature)) fail("InvalidArgument", "invalid time signature");
      if (c.time !== null && p.time < BEATS_EPSILON && c.time > BEATS_EPSILON) fail("InvalidArgument", "the time signature at beat 0 cannot move");
      tx.upsert("TimeSignature", { ...p, time: c.time !== null ? Math.max(0, c.time) : p.time, signature: c.signature ?? p.signature });
      break;
    }
    case "RemoveTimeSignatures":
      for (const id of c.ids) {
        const p = tx.get("TimeSignature", id) ?? fail("NotFound", `time signature ${id}`);
        if (p.time < BEATS_EPSILON) fail("InvalidArgument", "the time signature at beat 0 cannot be removed");
        tx.remove("TimeSignature", id);
      }
      break;
    case "SetMetronomeSettings": {
      const s = tx.project.settings;
      tx.setSettings({
        ...s,
        metronome_volume: c.volume !== null ? clamp(c.volume, -144, 6) : s.metronome_volume,
        metronome_accent: c.accent ?? s.metronome_accent,
        metronome_sound: c.sound ?? s.metronome_sound,
      });
      break;
    }
  }
}
