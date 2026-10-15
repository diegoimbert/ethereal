// Pure helpers of the indexed browser (tested in model.test.ts).
import type { BrowseLocation, BrowseRoot, BrowserQuery, BrowserRoot, BrowserSort, LibraryItem, LibraryItemKind } from "@/generated";
import { t, type MessageKey } from "@/i18n";
import { parentPath } from "../paths";

/** The "All" place (every root). */
export const ALL_ROOTS = "*all";

/** Page size of index queries. */
export const PAGE_SIZE = 100;

export type KindFilter = LibraryItemKind;

/** `label`: the chip; `search`: the search field's placeholder while the chip is on. */
export const KIND_FILTERS: ReadonlyArray<{ value: KindFilter; label: MessageKey; search: MessageKey }> = [
  { value: "Audio", label: "browser.kind.samples", search: "browser.search.samples" },
  { value: "Midi", label: "browser.kind.midi", search: "browser.search.midi" },
  { value: "Preset", label: "browser.kind.presets", search: "browser.search.presets" },
  { value: "Project", label: "browser.kind.projects", search: "browser.search.projects" },
];

export type ChosenSort = Exclude<BrowserSort, "Relevance">;

export const SORTS: ReadonlyArray<{ value: BrowserSort; label: MessageKey }> = [
  { value: "Name", label: "browser.sort.name" },
  { value: "Recent", label: "browser.sort.recent" },
  { value: "Duration", label: "browser.sort.duration" },
  { value: "Bpm", label: "browser.sort.bpm" },
  { value: "Relevance", label: "browser.sort.relevance" },
];

/** A place of the browser: a root (or `ALL_ROOTS`) and a folder inside it. */
export interface Place {
  root: string;
  /** Relative to the root (for a pack: relative to the pack folder). */
  folder: string;
}

/** Where a root can be browsed folder by folder (`Media::ListDirectory`), if anywhere. */
export interface RootLocation {
  location: BrowseLocation;
  /** The root's folder inside the location (`""`, or a pack's folder). */
  base: string;
}

export const joinPath = (a: string, b: string) => [a, b].filter(Boolean).join("/");

export function rootLocation(root: string, roots: ReadonlyArray<BrowserRoot>, locations: ReadonlyArray<BrowseRoot> | null): RootLocation | null {
  const r = roots.find((x) => x.id === root);
  if (!r || !locations) return null;
  const libraryLocation = (id: string) => locations.find((l) => l.location.type === "Library" && l.location.id === id)?.location ?? null;
  if (r.kind === "Pack") {
    const slash = r.id.indexOf("/");
    const location = slash > 0 ? libraryLocation(r.id.slice(0, slash)) : null;
    return location ? { location, base: r.id.slice(slash + 1) } : null;
  }
  if (r.kind === "Factory") return null;
  const location = libraryLocation(r.id);
  return location ? { location, base: "" } : null;
}

/**
 * Library roots shipped with the app (app text, lowercased with the UI): the factory presets
 * root (`BrowserRootKind::Factory`, id "factory") and the bundled factory samples ("Factory":
 * id "demo" on desktop, "browser" on the web).
 */
const FACTORY_ROOT_IDS: ReadonlySet<string> = new Set(["factory", "demo", "browser"]);
/** The built-in user library root: its name ("User library") is app text, its files are the user's. */
const USER_LIBRARY_ID = "user";

/** Whether root `id` (or the library root of pack id `"<root>/<dir>"`) holds factory content. */
export function isFactoryRoot(id: string): boolean {
  const slash = id.indexOf("/");
  return FACTORY_ROOT_IDS.has(slash > 0 ? id.slice(0, slash) : id);
}

/** Whether a root's name is user content (kept in its case): user folders, packs and configured folders, not factory or app-named roots. */
export function isUserRootName(r: { id: string; kind?: BrowserRoot["kind"] }): boolean {
  if (r.kind === "Factory" || isFactoryRoot(r.id)) return false;
  if (r.kind === "Folder" || r.kind === "Pack") return true;
  return r.id !== USER_LIBRARY_ID;
}

/** The built-in roots, named by the UI in the current language (the engine sends English names). */
const BUILTIN_ROOT_NAMES: Readonly<Record<string, MessageKey>> = {
  factory: "browser.root.factory_presets",
  demo: "browser.root.factory",
  browser: "browser.root.factory",
  [USER_LIBRARY_ID]: "browser.root.user_library",
};

/** A browse location's name: the built-in ones in the current language, the others as configured. */
export function locationName(l: BrowseRoot): string {
  if (l.location.type === "ProjectMedia") return t("browser.root.project_media");
  const key = BUILTIN_ROOT_NAMES[l.location.id];
  return key ? t(key) : l.name;
}

/** Tab label of a root: library roots keep their `Media::ListLocations` name. */
export function rootLabel(r: BrowserRoot, locations: ReadonlyArray<BrowseRoot> | null): string {
  // User folders can be renamed (`RenameFolder`): the index knows their name.
  if (r.kind === "Folder") return r.name;
  const key = BUILTIN_ROOT_NAMES[r.id];
  if (key) return t(key);
  const l = locations?.find((x) => x.location.type === "Library" && x.location.id === r.id);
  return l?.name ?? r.name;
}

export interface Filters {
  text: string;
  kind: KindFilter;
  tags: ReadonlyArray<string>;
  favouritesOnly: boolean;
  sort: ChosenSort;
}

/** The index query of a place and filters (first page). */
export function buildQuery(place: Place, f: Filters, roots: ReadonlyArray<BrowserRoot>): BrowserQuery {
  const text = f.text.trim();
  let scope: Pick<BrowserQuery, "roots" | "folder"> = { roots: [], folder: null };
  if (place.root !== ALL_ROOTS) {
    const r = roots.find((x) => x.id === place.root);
    // Pack folders are paths inside the pack's library root.
    const base = r?.kind === "Pack" ? r.id.slice(r.id.indexOf("/") + 1) : "";
    scope = { roots: [place.root], folder: place.folder ? joinPath(base, place.folder) : null };
  }
  return {
    text,
    kinds: [f.kind],
    tags: [...f.tags],
    favourites_only: f.favouritesOnly,
    ...scope,
    device: null,
    sort: text ? "Relevance" : f.sort,
    offset: 0,
    limit: PAGE_SIZE,
  };
}

/**
 * Folders on the results' paths whose name matches every word of `text` (search results
 * list them first, so a search can lead into a folder), as `{ root, path }` (path relative
 * to the root), unique, by name.
 */
export function matchingFolders(
  items: ReadonlyArray<LibraryItem>,
  text: string,
  under: Place,
  roots: ReadonlyArray<BrowserRoot>,
): { root: string; path: string; name: string }[] {
  const words = text.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  const scope = under.root === ALL_ROOTS ? null : placeToPath(under, roots);
  const out = new Map<string, { root: string; path: string; name: string }>();
  for (const item of items) {
    if (item.kind !== "Audio" && item.kind !== "Midi") continue;
    const parts = item.path.split("/").slice(0, -1);
    for (let i = 0; i < parts.length; i++) {
      const path = parts.slice(0, i + 1).join("/");
      const name = parts[i]!;
      if (scope && (scope.root !== item.root || !(scope.path === "" || path.startsWith(`${scope.path}/`)))) continue;
      if (!words.every((w) => name.toLowerCase().includes(w))) continue;
      out.set(`${item.root}/${path}`, { root: item.root, path, name });
    }
  }
  return [...out.values()].sort((a, b) => a.name.localeCompare(b.name) || a.path.localeCompare(b.path));
}

/** A place as a (library root, path inside it) pair. */
function placeToPath(p: Place, roots: ReadonlyArray<BrowserRoot>): { root: string; path: string } {
  const r = roots.find((x) => x.id === p.root);
  if (r?.kind === "Pack") {
    const slash = r.id.indexOf("/");
    return { root: r.id.slice(0, slash), path: joinPath(r.id.slice(slash + 1), p.folder) };
  }
  return { root: p.root, path: p.folder };
}

/** Secondary text of a result row: its folder, a preset's device, or "Project". */
export function itemDetail(item: LibraryItem): string {
  if (item.kind === "Project") return t("browser.item.project");
  if (item.kind === "Preset") return presetDeviceKey(item)?.replace(/-/g, " ") ?? "";
  return parentPath(item.path);
}

/** The device key a preset item is for (`"poly-synth"`), from its path. */
export function presetDeviceKey(item: Pick<LibraryItem, "root" | "path" | "kind">): string | null {
  if (item.kind !== "Preset") return null;
  const path = item.path.replace(/^Presets\//, "");
  const key = path.split("/")[0];
  return key && key !== "plugins" && path.includes("/") ? key : null;
}

/** `BuiltinDeviceType::key()`: "PolySynth" → "poly-synth". */
export const builtinKey = (t: string) => t.replace(/(?!^)([A-Z])/g, "-$1").toLowerCase();

/** `"0.6 s"`, `"12 s"`, `"1:05"`. */
export function formatDuration(seconds: number): string {
  if (seconds < 10) return `${seconds.toFixed(1)} s`;
  if (seconds < 60) return `${Math.round(seconds)} s`;
  const s = Math.round(seconds);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Short key label: "A minor" → "Am", "C# major" → "C#". */
export function shortKey(key: string): string {
  const m = /^([A-G][#b]?)\s*(minor|major)?$/i.exec(key.trim());
  if (!m) return key;
  return m[2]?.toLowerCase() === "minor" ? `${m[1]}m` : m[1]!;
}

/** "punchy, Acoustic ,, punchy" → ["acoustic", "punchy"] (the engine normalizes too). */
export function parseTags(input: string): string[] {
  return [
    ...new Set(
      input
        .split(",")
        .map((t) => t.trim().toLowerCase())
        .filter(Boolean),
    ),
  ].sort();
}

const SYNC_KEY = "eth-browser-sync";

/** Tempo-synced preview: remembered per browser, on by default. */
export function loadSync(): boolean {
  try {
    return localStorage.getItem(SYNC_KEY) !== "0";
  } catch {
    return true;
  }
}

export function saveSync(on: boolean): void {
  try {
    localStorage.setItem(SYNC_KEY, on ? "1" : "0");
  } catch {
    // Storage unavailable: the choice lasts for this session only.
  }
}
