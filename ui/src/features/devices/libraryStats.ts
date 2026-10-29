/**
 * The device library's favourites and usage (for "Most used" / "Most recent"), per device
 * type (`libraryKey`), its sort and the plugin formats it hides, remembered across sessions
 * (localStorage; this session only where storage is unavailable). Usage counts every add from
 * the library (`libraryInsert.ts`).
 */
import { create } from "zustand";
import type { PluginFormat } from "@/generated";
import type { LibraryDevicePayload } from "./libraryDevice";

export type LibrarySort = "Name" | "MostUsed" | "Recent";

interface Usage {
  count: number;
  /** Wall clock of the last use (ms). */
  last: number;
}

interface Saved {
  favourites: string[];
  usage: Record<string, Usage>;
  /** The chosen sort (default "MostUsed"). */
  librarySort: LibrarySort;
  /** The sort before "MostUsed" became the default: "Name" was the default then, so only a different one was chosen. */
  sort?: LibrarySort;
  /** Plugin formats toggled off in the library (every format is shown by default). */
  hiddenFormats: PluginFormat[];
}

interface LibraryStats {
  favourites: ReadonlySet<string>;
  usage: Readonly<Record<string, Usage>>;
  sort: LibrarySort;
  hiddenFormats: ReadonlySet<PluginFormat>;
  toggleFavourite(key: string): void;
  recordUse(key: string): void;
  setSort(sort: LibrarySort): void;
  setFormatShown(format: PluginFormat, shown: boolean): void;
}

const KEY = "eth-device-library";
const FORMATS: ReadonlySet<string> = new Set<PluginFormat>(["Clap", "Vst3", "Vst2", "Au"]);

/**
 * The stable key of a library device: its built-in type, or its plugin (format + id, plus the
 * name when another listed plugin shares them: a VST2 uniqueID collision).
 */
export function libraryKey(p: Pick<LibraryDevicePayload, "device">): string {
  if (p.device.type === "Builtin") return `builtin:${p.device.builtin}`;
  const key = `plugin:${p.device.format}:${p.device.plugin_id}`;
  return p.device.shared ? `${key}:${p.device.name ?? ""}` : key;
}

const SORTS: ReadonlySet<string> = new Set<LibrarySort>(["Name", "MostUsed", "Recent"]);

const EMPTY: Saved = { favourites: [], usage: {}, librarySort: "MostUsed", hiddenFormats: [] };

/** The saved library state (exported for tests). */
export function load(): Saved {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Saved>;
    return {
      favourites: Array.isArray(v.favourites) ? v.favourites.filter((x) => typeof x === "string") : [],
      usage: v.usage && typeof v.usage === "object" ? v.usage : {},
      librarySort: v.librarySort && SORTS.has(v.librarySort) ? v.librarySort : v.sort === "Recent" ? "Recent" : "MostUsed",
      hiddenFormats: Array.isArray(v.hiddenFormats) ? v.hiddenFormats.filter((f) => FORMATS.has(f)) : [],
    };
  } catch {
    return EMPTY;
  }
}

function save(s: Pick<LibraryStats, "favourites" | "usage" | "sort" | "hiddenFormats">): void {
  try {
    const saved: Saved = { favourites: [...s.favourites], usage: s.usage, librarySort: s.sort, hiddenFormats: [...s.hiddenFormats] };
    localStorage.setItem(KEY, JSON.stringify(saved));
  } catch {
    // Storage unavailable: this session only.
  }
}

const initial = load();

export const useLibraryStats = create<LibraryStats>()((set, get) => ({
  favourites: new Set(initial.favourites),
  usage: initial.usage,
  sort: initial.librarySort,
  hiddenFormats: new Set(initial.hiddenFormats),
  toggleFavourite: (key) => {
    const next = new Set(get().favourites);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    set({ favourites: next });
    save(get());
  },
  recordUse: (key) => {
    const prev = get().usage[key];
    set({ usage: { ...get().usage, [key]: { count: (prev?.count ?? 0) + 1, last: Date.now() } } });
    save(get());
  },
  setSort: (sort) => {
    set({ sort });
    save(get());
  },
  setFormatShown: (format, shown) => {
    const next = new Set(get().hiddenFormats);
    if (shown) next.delete(format);
    else next.add(format);
    set({ hiddenFormats: next });
    save(get());
  },
}));

/** Count one use of a library device. */
export const recordDeviceUse = (p: Pick<LibraryDevicePayload, "device">) => useLibraryStats.getState().recordUse(libraryKey(p));

/** Tests: forget everything. */
export function resetLibraryStats(): void {
  useLibraryStats.setState({ favourites: new Set(), usage: {}, sort: "Name", hiddenFormats: new Set() });
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* nothing saved */
  }
}
