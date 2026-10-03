/**
 * MPE settings model (v0.3, `mpe`; Rust `ether_model::expression::{MpeSettings, check_mpe}`,
 * CONTRACTS.md §13.3). Pure functions shared by the inspector and the MockTransport
 * (`transport/mock/roadmap/mpe.ts`), so the mock validates exactly like the engine.
 *
 * A MIDI track with `mpe` set reads its MIDI input as MPE: every member channel carries one
 * note, and that channel's pitch bend (over `note_pitch_range` semitones), channel pressure
 * and CC 74 become the note's `Pitch` / `Pressure` / `Timbre` expressions. The master
 * channel (channel 1 for the lower zone, 16 for the upper) stays channel-wide.
 */

import type { MpeSettings, MpeZone } from "@/generated";

/** MPE's defaults: lower zone, 15 member channels, ±48 semitones per note, ±2 master. */
export const DEFAULT_MPE: MpeSettings = {
  zone: "Lower",
  member_channels: 15,
  note_pitch_range: 48,
  master_pitch_range: 2,
};

export const MAX_MEMBER_CHANNELS = 15;
export const MAX_PITCH_RANGE = 96;

/** `ether_model::expression::check_mpe`: an error message, or `null` when valid. */
export function checkMpe(m: MpeSettings): string | null {
  if (!(Number.isInteger(m.member_channels) && m.member_channels >= 1 && m.member_channels <= MAX_MEMBER_CHANNELS)) {
    return "MPE member channels must be 1..=15";
  }
  if (!(Number.isFinite(m.note_pitch_range) && m.note_pitch_range >= 1 && m.note_pitch_range <= MAX_PITCH_RANGE)) {
    return "MPE note pitch range must be 1..=96 semitones";
  }
  if (!(Number.isFinite(m.master_pitch_range) && m.master_pitch_range >= 0 && m.master_pitch_range <= MAX_PITCH_RANGE)) {
    return "MPE master pitch range must be 0..=96 semitones";
  }
  return null;
}

export function sameMpe(a: MpeSettings | null | undefined, b: MpeSettings | null | undefined): boolean {
  if (!a || !b) return !a && !b;
  return (
    a.zone === b.zone &&
    a.member_channels === b.member_channels &&
    a.note_pitch_range === b.note_pitch_range &&
    a.master_pitch_range === b.master_pitch_range
  );
}

/** The master channel (1-based, as shown to users). */
export function masterChannel(zone: MpeZone): number {
  return zone === "Lower" ? 1 : 16;
}

/** The member channels (1-based, inclusive): lower `2..=n+1`, upper `16-n..=15`. */
export function memberChannels(m: Pick<MpeSettings, "zone" | "member_channels">): [number, number] {
  const n = Math.min(MAX_MEMBER_CHANNELS, Math.max(1, Math.round(m.member_channels)));
  return m.zone === "Lower" ? [2, n + 1] : [16 - n, 15];
}

/** "Channels 2–16" style summary of a zone. */
export function zoneSummary(m: MpeSettings): string {
  const [a, b] = memberChannels(m);
  return `Master ${masterChannel(m.zone)} · notes on ${a === b ? `${a}` : `${a}–${b}`}`;
}

/**
 * Semitones shown by a per-note pitch lane: the track's per-note bend range with MPE (what
 * a controller can reach), else an octave either way (wide enough for drawn glides).
 */
export function pitchWindow(mpe: MpeSettings | null | undefined): number {
  return mpe ? mpe.note_pitch_range : 12;
}
