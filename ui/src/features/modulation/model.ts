/**
 * Modulation model helpers (mirror `ether_model::modulation` scope rules, CONTRACTS.md
 * §12.6) and the app-wide "mapping in progress" state: a source picked from a modulator
 * (map button or drag handle) or a rack macro, waiting for a target param.
 */

import { create } from "zustand";
import type { Device, DeviceId, ModMapping, ModSource, Modulator, Project } from "@/generated";

/** HTML drag type of a modulation source (JSON `ModSource`). */
export const MOD_DRAG_TYPE = "application/x-ethereal-mod-source";
/** Depth of a new mapping (normalized units). */
export const DEFAULT_DEPTH = 0.5;
/** Rack macro params `0..MACROS`; the chain selector follows. */
export const MACROS = 8;
export const SELECTOR_PARAM = 8;

const RACK_TYPES = new Set(["InstrumentRack", "AudioEffectRack", "MidiEffectRack"]);

export function isChainRack(d: Device | undefined | null): boolean {
  return !!d && d.kind.type === "Builtin" && RACK_TYPES.has(d.kind.device.type);
}

/** Whether `d` may host modulators (a track-chain device). */
export function canHostModulators(d: Device): boolean {
  return d.pad === null && d.chain == null;
}

export function sameSource(a: ModSource, b: ModSource): boolean {
  if (a.type === "Modulator" && b.type === "Modulator") return a.modulator === b.modulator;
  if (a.type === "Macro" && b.type === "Macro") return a.rack === b.rack && a.index === b.index;
  return false;
}

/** The device a source lives in (modulator host or macro rack). */
export function sourceHost(project: Project, s: ModSource): DeviceId | null {
  return s.type === "Modulator" ? (project.modulators[s.modulator]?.device ?? null) : s.rack;
}

/** Rack of the chain `d` sits on. */
export function rackOf(project: Project, d: Device): DeviceId | null {
  return d.chain != null ? (project.rack_chains[d.chain]?.rack ?? null) : null;
}

/** Scope rules: can `source` modulate `device`'s `param` (and is it not mapped yet)? */
export function canTarget(project: Project, source: ModSource, device: Device, param: number): boolean {
  const host = sourceHost(project, source);
  if (host === null) return false;
  if (source.type === "Modulator") {
    if (host !== device.id && rackOf(project, device) !== host) return false;
    if (host === device.id && isChainRack(device) && param < SELECTOR_PARAM) return false;
  } else {
    const inRack = rackOf(project, device) === source.rack;
    const own = device.id === source.rack && param >= SELECTOR_PARAM;
    if (!inRack && !own) return false;
  }
  return !Object.values(project.mod_mappings).some((m) => sameSource(m.source, source) && m.device === device.id && m.param === param);
}

/** Mappings targeting `device`'s `param`, sorted by id. */
export function mappingsOf(table: Record<string, ModMapping> | undefined, device: DeviceId, param: number): ModMapping[] {
  if (!table) return [];
  return Object.values(table)
    .filter((m) => m.device === device && m.param === param)
    .sort((a, b) => (a.id < b.id ? -1 : 1));
}

/** Modulators of a device in order. */
export function modulatorsOf(table: Record<string, Modulator> | undefined, device: DeviceId): Modulator[] {
  if (!table) return [];
  return Object.values(table)
    .filter((m) => m.device === device)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1));
}

/** Display name of a source ("LFO", "Macro 3"). */
export function sourceName(project: Project, s: ModSource): string {
  if (s.type === "Modulator") return project.modulators[s.modulator]?.name ?? "Modulator";
  return `Macro ${s.index + 1}`;
}

/** Stable colour slot (0..3) of a source, for its rings and chips. */
export function sourceSlot(project: Project, s: ModSource): number {
  if (s.type === "Macro") return s.index % 4;
  const m = project.modulators[s.modulator];
  if (!m) return 0;
  return Math.max(0, modulatorsOf(project.modulators, m.device).findIndex((x) => x.id === m.id)) % 4;
}

interface MappingState {
  /** The source waiting for a target (click a highlighted param), or `null`. */
  source: ModSource | null;
  start(source: ModSource): void;
  stop(): void;
}

export const useModMapping = create<MappingState>()((set) => ({
  source: null,
  start: (source) => set({ source }),
  stop: () => set({ source: null }),
}));
