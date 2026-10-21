/**
 * Which scanned plugins the device library lists (Settings > Plugins > Plugin list), kept on
 * this device (localStorage; this session only where storage is unavailable):
 * - one format per plugin (on by default): a plugin installed as CLAP, VST3, AU and VST2 is
 *   listed once, in the preferred format (CLAP, then VST3, AU, VST2). Off, every format shows
 *   (to pick a specific one);
 * - hide rules: a plugin (its name and vendor) in every format, or in one format only (e.g.
 *   hide a plugin's CLAP version to get its VST3 one listed).
 * The library's format toggles apply first: turned to VST3 only, it lists the VST3 versions.
 * Lists that need the exact plugin (the import resolver, Replace plugin) list every format.
 */
import { create } from "zustand";
import type { PluginDescriptor, PluginFormat } from "@/generated";

/** Hides the plugin named `name` by `vendor`: in `format` only, or every format (`null`). */
export interface PluginHideRule {
  name: string;
  vendor: string;
  format: PluginFormat | null;
}

export interface PluginVisibility {
  /** One format per plugin. */
  dedupe: boolean;
  /** The format listed first when a plugin has several (then the others in `DEDUPE_ORDER`). */
  preferred: PluginFormat;
  rules: PluginHideRule[];
}

/** The default preference: CLAP, then VST3, AU, VST2. */
export const DEDUPE_ORDER: readonly PluginFormat[] = ["Clap", "Vst3", "Au", "Vst2"];

export const DEFAULT_VISIBILITY: PluginVisibility = { dedupe: true, preferred: "Clap", rules: [] };

/** A name or vendor as compared across formats: lowercase letters and digits ("Serum (x64)" is "serum"). */
const norm = (s: string) =>
  s
    .toLowerCase()
    .replace(/\((x64|x86|64[- ]?bit|32[- ]?bit|stereo|mono)\)/g, "")
    .replace(/[^\p{L}\p{N}]/gu, "");

/** The same plugin in every format: its vendor, name and kind. */
export const pluginIdentity = (p: Pick<PluginDescriptor, "name" | "vendor" | "category">) => `${norm(p.vendor)}|${norm(p.name)}|${p.category}`;

/** Whether `rule` hides `p`. */
export const hides = (rule: PluginHideRule, p: Pick<PluginDescriptor, "name" | "vendor" | "format">) =>
  norm(rule.name) === norm(p.name) && norm(rule.vendor) === norm(p.vendor) && (rule.format === null || rule.format === p.format);

/** The formats in preference order. */
export const preferenceOrder = (preferred: PluginFormat): PluginFormat[] => [preferred, ...DEDUPE_ORDER.filter((f) => f !== preferred)];

/** The plugins listed: the hidden ones out, then (deduplicating) each plugin in its preferred format. */
export function visiblePlugins<P extends Pick<PluginDescriptor, "name" | "vendor" | "format" | "category">>(plugins: readonly P[], v: PluginVisibility): P[] {
  const shown = plugins.filter((p) => !v.rules.some((r) => hides(r, p)));
  if (!v.dedupe) return shown;
  const order = preferenceOrder(v.preferred);
  const best = new Map<string, PluginFormat>();
  for (const p of shown) {
    const id = pluginIdentity(p);
    const was = best.get(id);
    if (was === undefined || order.indexOf(p.format) < order.indexOf(was)) best.set(id, p.format);
  }
  return shown.filter((p) => best.get(pluginIdentity(p)) === p.format);
}

const KEY = "eth.plugins.visibility";
const FORMATS: ReadonlySet<string> = new Set(DEDUPE_ORDER);

function load(): PluginVisibility {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<PluginVisibility>;
    return {
      dedupe: typeof v.dedupe === "boolean" ? v.dedupe : DEFAULT_VISIBILITY.dedupe,
      preferred: typeof v.preferred === "string" && FORMATS.has(v.preferred) ? v.preferred : DEFAULT_VISIBILITY.preferred,
      rules: Array.isArray(v.rules)
        ? v.rules.filter(
            (r): r is PluginHideRule =>
              !!r &&
              typeof r.name === "string" &&
              typeof r.vendor === "string" &&
              (r.format === null || (typeof r.format === "string" && FORMATS.has(r.format))),
          )
        : [],
    };
  } catch {
    return DEFAULT_VISIBILITY;
  }
}

function save(v: PluginVisibility) {
  try {
    localStorage.setItem(KEY, JSON.stringify(v));
  } catch {
    // Storage unavailable: this session only.
  }
}

interface VisibilityStore extends PluginVisibility {
  setDedupe(dedupe: boolean): void;
  setPreferred(format: PluginFormat): void;
  /** Adds a hide rule (no duplicate); returns whether it was new. */
  hide(rule: PluginHideRule): boolean;
  /** Replaces rule `i`. */
  setRule(i: number, rule: PluginHideRule): void;
  removeRule(i: number): void;
  /** Back to the defaults (one format per plugin, CLAP first, nothing hidden). */
  reset(): void;
  /** Re-reads the stored settings (tests). */
  reload(): void;
}

const pick = (s: PluginVisibility): PluginVisibility => ({ dedupe: s.dedupe, preferred: s.preferred, rules: s.rules });

export const usePluginVisibility = create<VisibilityStore>((set, get) => {
  const update = (patch: Partial<PluginVisibility>) => {
    set(patch);
    save(pick(get()));
  };
  const same = (a: PluginHideRule, b: PluginHideRule) => norm(a.name) === norm(b.name) && norm(a.vendor) === norm(b.vendor) && a.format === b.format;
  return {
    ...load(),
    setDedupe: (dedupe) => update({ dedupe }),
    setPreferred: (preferred) => update({ preferred }),
    hide: (rule) => {
      if (get().rules.some((r) => same(r, rule))) return false;
      update({ rules: [...get().rules, rule] });
      return true;
    },
    setRule: (i, rule) => update({ rules: get().rules.map((r, j) => (j === i ? rule : r)) }),
    removeRule: (i) => update({ rules: get().rules.filter((_, j) => j !== i) }),
    reset: () => update({ ...DEFAULT_VISIBILITY }),
    reload: () => set(load()),
  };
});
