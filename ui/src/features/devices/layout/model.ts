/**
 * Pure layout model of the shared device renderer (CONTRACTS.md §12.4.2): which params a
 * widget binds, the generic layout for devices without one, and how a descriptor resolves
 * to what the panel shows ("main" sections always, "more" behind the disclosure).
 */

import type { DeviceDescriptor, DeviceLayout, LayoutItem, LayoutSection, ParamId, ParamInfo, Widget } from "@/generated";
import { groupParams, splitMainParams, type ParamGroup } from "../chainUtils";

export { FOLD_OVER, MAIN_MAX, groupParams, splitMainParams } from "../chainUtils";

/** Params a widget binds (mirror of `ether-devices/tests/layouts.rs::bound`). */
export function boundParams(w: Widget): ParamId[] {
  const some = (xs: ReadonlyArray<ParamId | null | undefined>) => xs.filter((x): x is ParamId => x != null);
  switch (w.type) {
    case "Knob":
    case "Slider":
    case "Toggle":
    case "Choice":
    case "Number":
      return [w.param];
    case "Envelope":
      return some([w.attack, w.decay, w.sustain, w.release, w.delay, w.hold]);
    case "FilterCurve":
      return some([w.cutoff, w.resonance, w.mode, w.drive, w.gain]);
    case "TransferCurve":
      return some([w.drive, w.curve, w.bias]);
    case "Oscillator":
      return some([w.shape, w.position]);
    case "Lfo":
      return some([w.shape, w.rate, w.amount]);
    case "StepEditor":
      return Array.from({ length: w.count }, (_, i) => w.first + i);
    case "XyPad":
      return [w.x, w.y];
    case "Crossover":
      return [...w.frequencies];
    case "SampleWaveform":
      return some([w.start, w.end]);
    case "EqCurve":
      return [...w.bands.flatMap((b) => some([b.on, b.kind, b.freq, b.gain, b.q])), ...w.crossovers];
    case "Macros":
      // The 8 macro knobs of a rack are params 0..8 (CONTRACTS.md §12.6).
      return [0, 1, 2, 3, 4, 5, 6, 7];
    case "ZoneMap":
    case "HardwareRouting":
    case "Spectrum":
    case "Tuner":
    case "Meter":
    case "RackChains":
      return [];
  }
}

/** Every param referenced anywhere in `layout`. */
export function referencedParams(layout: DeviceLayout): Set<ParamId> {
  const out = new Set<ParamId>();
  for (const s of layout.sections) for (const it of s.items) for (const p of boundParams(it.widget)) out.add(p);
  return out;
}

type Group = ParamGroup;

/** Is `info` an on/off param (two labels, or a `Toggle` unit without labels)? */
export function isToggleParam(info: ParamInfo): boolean {
  return info.labels?.length === 2 || (info.unit === "Toggle" && !info.labels);
}

/** The param widget the generic layout uses for `info` (toggle, choice or knob). */
export function genericWidget(info: ParamInfo): Widget {
  if (isToggleParam(info)) return { type: "Toggle", param: info.id };
  if (info.labels && info.labels.length > 2) return { type: "Choice", param: info.id };
  return { type: "Knob", param: info.id };
}

/** Section id prefix of generic sections (the renderer keeps their auto-fill grid). */
export const GENERIC_PREFIX = "generic:";

export function isGenericSection(s: LayoutSection): boolean {
  return s.id.startsWith(GENERIC_PREFIX);
}

function genericSections(groups: ReadonlyArray<Group>, size: LayoutItem["size"], tag: string): LayoutSection[] {
  return groups.map((g, i) => ({
    id: `${GENERIC_PREFIX}${tag}:${i}:${g.group ?? ""}`,
    title: g.group,
    span: 1,
    columns: 2,
    items: g.params.map((p) => ({ widget: genericWidget(p), size, colspan: 1, label: null })),
  }));
}

/** What a device panel shows: `main` always, `more` behind the "More controls" disclosure. */
export interface ResolvedLayout {
  main: DeviceLayout;
  /** `null` = nothing folded. */
  more: DeviceLayout | null;
  /** Params inside `more`. */
  moreCount: number;
  /** The device ships its own layout (else: the generic one). */
  declared: boolean;
  /**
   * The card shows only some of the params (`PARAM_CARD_CAP` for plugins); the rest are
   * reached through "Show all N parameters…" (a searchable, windowed list).
   */
  capped: boolean;
  /** Visible (non-hidden) params of the device. */
  total: number;
}

/**
 * Most controls a plugin card renders by default (pinned params first, then the plugin's
 * own quick controls, then its first automatable params). The rest live in the
 * "Show all parameters" list, which only mounts the rows in view.
 */
export const PARAM_CARD_CAP = 16;
/**
 * Any device (built-ins included) with more visible params than this gets the capped card,
 * and a declared layout folds at most this many under "More" (beyond: the full list).
 */
export const UNCAPPED_MAX = 64;

const NO_PINS: ReadonlyArray<ParamId> = [];

/**
 * The params a capped card shows, in order: `pins` (the user's, per plugin), the plugin's
 * own quick controls (`ParamInfo.remote`, CLAP remote-controls pages), then the first
 * automatable visible params, then any visible ones; at most `cap`, each once.
 */
export function cardParams(params: ReadonlyArray<ParamInfo>, pins: ReadonlyArray<ParamId> = NO_PINS, cap = PARAM_CARD_CAP): ParamInfo[] {
  const out: ParamInfo[] = [];
  const taken = new Set<ParamId>();
  const take = (p: ParamInfo | undefined) => {
    if (!p || p.hidden || taken.has(p.id) || out.length >= cap) return;
    taken.add(p.id);
    out.push(p);
  };
  if (pins.length) {
    const byId = new Map(params.map((p) => [p.id, p]));
    for (const id of pins) take(byId.get(id));
  }
  if (out.length < cap) {
    const remote = params.filter((p) => p.remote != null).sort((a, b) => a.remote! - b.remote!);
    for (const p of remote) take(p);
  }
  for (const p of params) {
    if (out.length >= cap) break;
    if (p.automatable) take(p);
  }
  for (const p of params) {
    if (out.length >= cap) break;
    take(p);
  }
  return out;
}

function visibleCount(params: ReadonlyArray<ParamInfo>): number {
  let n = 0;
  for (const p of params) if (!p.hidden) n++;
  return n;
}

/**
 * The generic layout: params grouped by `ParamInfo.group`; the leading groups are the
 * large main controls and the rest folds under "More" (`splitMainParams`).
 */
export function genericLayout(params: ReadonlyArray<ParamInfo>): ResolvedLayout {
  const { main, more } = splitMainParams(groupParams(params));
  const moreCount = more.reduce((n, g) => n + g.params.length, 0);
  return {
    main: { sections: genericSections(main, "Large", "main") },
    more: more.length ? { sections: genericSections(more, "Medium", "more") } : null,
    moreCount,
    declared: false,
    capped: false,
    total: visibleCount(params),
  };
}

/**
 * Resolve a descriptor: its declared layout (plus the visible params it doesn't reference,
 * grouped, behind "More") or the generic layout. A plugin with more than `PARAM_CARD_CAP`
 * visible params (any device above `UNCAPPED_MAX`) gets the capped card: the generic
 * layout of `cardParams(params, pins)` only.
 */
export function resolveLayout(
  descriptor: Pick<DeviceDescriptor, "params" | "layout"> & Partial<Pick<DeviceDescriptor, "device_type">>,
  pins: ReadonlyArray<ParamId> = NO_PINS,
): ResolvedLayout {
  const layout = descriptor.layout;
  const total = visibleCount(descriptor.params);
  if (!layout || layout.sections.length === 0) {
    const cap = descriptor.device_type?.type === "Plugin" ? PARAM_CARD_CAP : UNCAPPED_MAX;
    if (total <= cap) return genericLayout(descriptor.params);
    return { ...genericLayout(cardParams(descriptor.params, pins)), capped: true, total };
  }
  const used = referencedParams(layout);
  const rest = groupParams(descriptor.params.filter((p) => !used.has(p.id)));
  const moreCount = rest.reduce((n, g) => n + g.params.length, 0);
  const capped = moreCount > UNCAPPED_MAX;
  return {
    main: layout,
    more: rest.length && !capped ? { sections: genericSections(rest, "Medium", "more") } : null,
    moreCount: capped ? 0 : moreCount,
    declared: true,
    capped,
    total,
  };
}
