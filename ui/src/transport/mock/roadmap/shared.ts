/**
 * Helpers shared by several roadmap v2 mock simulations (contracts-2, base-17): the
 * cascades the core reducer runs when a device, send or track is deleted (MIDI mappings
 * and sidechains that point at it). Owned by contracts-2 / base; feature nodes only add
 * here through the manager.
 */

import type { MidiMapping, TrackId } from "@/generated";
import type { ReducerContext } from "../documentReducer";

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Sort by fractional order key (ties by id), like the Rust chain order. */
export function byOrder<T extends { order: string; id: string }>(a: T, b: T): number {
  if (a.order !== b.order) return a.order < b.order ? -1 : 1;
  return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
}

/** `true` if a mapping target references `track` directly. */
export function mappingRefsTrack(m: MidiMapping, track: TrackId): boolean {
  const t = m.target;
  if (t.type === "Param") return (t.target.type === "TrackVolume" || t.target.type === "TrackPan") && t.target.track === track;
  return (t.type === "TrackMute" || t.type === "TrackSolo" || t.type === "TrackArm") && t.track === track;
}

export function deleteMappingsWhere(ctx: ReducerContext, pred: (m: MidiMapping) => boolean): void {
  for (const m of ctx.tx.all("MidiMapping")) if (pred(m)) ctx.tx.remove("MidiMapping", m.id);
}

/** Device deleted: its MIDI mappings go. */
export function onDeviceDeleted(ctx: ReducerContext, device: string): void {
  deleteMappingsWhere(ctx, (m) => m.target.type === "Param" && m.target.target.type === "DeviceParam" && m.target.target.device === device);
}

/** Send deleted: its MIDI mappings go. */
export function onSendDeleted(ctx: ReducerContext, send: string): void {
  deleteMappingsWhere(ctx, (m) => m.target.type === "Param" && m.target.target.type === "SendLevel" && m.target.target.send === send);
}

/** Track deleted: its MIDI mappings go and sidechains listening to it are cut. */
export function onTrackDeleted(ctx: ReducerContext, track: TrackId): void {
  deleteMappingsWhere(ctx, (m) => mappingRefsTrack(m, track));
  for (const d of ctx.tx.all("Device")) if (d.sidechain === track) ctx.tx.upsert("Device", { ...d, sidechain: null });
}
