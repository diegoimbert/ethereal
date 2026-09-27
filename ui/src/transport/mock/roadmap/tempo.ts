/**
 * Mock of `Tempo::*` (tempo map CRUD + metronome settings). Owned by `tempo-metronome`;
 * mirrors `crates/ether-controller/src/tempo/mod.rs`:
 * - times are finite and >= 0; the points at beat 0 can be edited but not moved or removed;
 *   two tempo points (or two time signatures) never share a position;
 * - BPM is clamped to 20..=999, metronome volume to -144..=+6 dB;
 * - a time-signature change must fall on a bar line of the signature in effect before it;
 * - edits that change nothing produce no patch.
 */

import type { Beats, Project, TempoCommand, TimeSignature } from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";
import { fail, type ReducerContext } from "../documentReducer";
import { clamp } from "./shared";

function validSignature(sig: TimeSignature): boolean {
  return Number.isInteger(sig.numerator) && sig.numerator >= 1 && sig.numerator <= 99 && [1, 2, 4, 8, 16, 32].includes(sig.denominator);
}

const near = (a: number, b: number) => Math.abs(a - b) <= BEATS_EPSILON;

function checkTime(what: string, t: Beats): void {
  if (!Number.isFinite(t) || t < 0) fail("InvalidArgument", `${what} time must be finite and >= 0`);
}

function checkBpm(bpm: number): number {
  if (!Number.isFinite(bpm)) fail("InvalidArgument", "bpm must be finite");
  return clamp(bpm, 20, 999);
}

function checkFreeTempoTime(p: Project, t: Beats, except: string | null): void {
  if (Object.values(p.tempo_points).some((q) => q.id !== except && near(q.time, t))) {
    fail("InvalidArgument", `there is already a tempo point at beat ${t}`);
  }
}

function checkSignatureTime(p: Project, t: Beats, except: string | null): void {
  const sigs = Object.values(p.time_signatures).filter((s) => s.id !== except);
  if (sigs.some((s) => near(s.time, t))) fail("InvalidArgument", `there is already a time signature at beat ${t}`);
  const prev = sigs.filter((s) => s.time < t).sort((a, b) => a.time - b.time).pop();
  if (!prev) return;
  const bar = (prev.signature.numerator * 4) / prev.signature.denominator;
  const bars = (t - prev.time) / bar;
  if (Math.abs((bars - Math.round(bars)) * bar) > BEATS_EPSILON) fail("InvalidArgument", "a time signature change must fall on a bar line");
}

export function tempoCommand(ctx: ReducerContext, c: TempoCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "AddTempoPoint": {
      if (tx.get("TempoPoint", c.id)) return;
      checkTime("tempo point", c.time);
      const bpm = checkBpm(c.bpm);
      checkFreeTempoTime(tx.project, c.time, null);
      tx.upsert("TempoPoint", { id: c.id, time: c.time, bpm, curve: c.curve });
      break;
    }
    case "EditTempoPoint": {
      const p = tx.get("TempoPoint", c.id) ?? fail("NotFound", `tempo point ${c.id}`);
      let time = p.time;
      if (c.time !== null && !near(c.time, p.time)) {
        checkTime("tempo point", c.time);
        if (near(p.time, 0)) fail("InvalidArgument", "the tempo point at beat 0 cannot be moved");
        checkFreeTempoTime(tx.project, c.time, p.id);
        time = c.time;
      }
      const bpm = c.bpm !== null ? checkBpm(c.bpm) : p.bpm;
      const curve = c.curve ?? p.curve;
      if (time !== p.time || bpm !== p.bpm || curve !== p.curve) tx.upsert("TempoPoint", { ...p, time, bpm, curve });
      break;
    }
    case "RemoveTempoPoints":
      for (const id of c.ids) {
        const p = tx.get("TempoPoint", id) ?? fail("NotFound", `tempo point ${id}`);
        if (near(p.time, 0)) fail("InvalidArgument", "the tempo point at beat 0 cannot be removed");
        tx.remove("TempoPoint", id);
      }
      break;
    case "AddTimeSignature":
      if (tx.get("TimeSignature", c.id)) return;
      checkTime("time signature", c.time);
      if (!validSignature(c.signature)) fail("InvalidArgument", "invalid time signature");
      checkSignatureTime(tx.project, c.time, null);
      tx.upsert("TimeSignature", { id: c.id, time: c.time, signature: c.signature });
      break;
    case "EditTimeSignature": {
      const p = tx.get("TimeSignature", c.id) ?? fail("NotFound", `time signature ${c.id}`);
      if (c.signature !== null && !validSignature(c.signature)) fail("InvalidArgument", "invalid time signature");
      let time = p.time;
      if (c.time !== null && !near(c.time, p.time)) {
        checkTime("time signature", c.time);
        if (near(p.time, 0)) fail("InvalidArgument", "the time signature at beat 0 cannot be moved");
        checkSignatureTime(tx.project, c.time, p.id);
        time = c.time;
      }
      const signature = c.signature ?? p.signature;
      const same = signature.numerator === p.signature.numerator && signature.denominator === p.signature.denominator;
      if (time !== p.time || !same) tx.upsert("TimeSignature", { ...p, time, signature });
      break;
    }
    case "RemoveTimeSignatures":
      for (const id of c.ids) {
        const p = tx.get("TimeSignature", id) ?? fail("NotFound", `time signature ${id}`);
        if (near(p.time, 0)) fail("InvalidArgument", "the time signature at beat 0 cannot be removed");
        tx.remove("TimeSignature", id);
      }
      break;
    case "SetMetronomeSettings": {
      const s = tx.project.settings;
      if (c.volume !== null && !Number.isFinite(c.volume)) fail("InvalidArgument", "metronome volume must be finite");
      const next = {
        ...s,
        metronome_volume: c.volume !== null ? clamp(c.volume, -144, 6) : s.metronome_volume,
        metronome_accent: c.accent ?? s.metronome_accent,
        metronome_sound: c.sound ?? s.metronome_sound,
      };
      if (
        next.metronome_volume !== s.metronome_volume ||
        next.metronome_accent !== s.metronome_accent ||
        next.metronome_sound !== s.metronome_sound
      ) {
        tx.setSettings(next);
      }
      break;
    }
  }
}
