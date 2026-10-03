/**
 * Mock of `Browser::*` (v0.2, owned by `browser-v2`; CONTRACTS.md §12.8): an in-memory index
 * over the mock library (`library.ts`) plus the factory presets, with the engine's query
 * rules (`crates/ether-controller/src/browser`):
 * - ids `"<root>/<path>"`; roots: the library (`LIBRARY_ID`), its sample packs (top-level
 *   folders in `MOCK_PACKS`, as if they held a `pack.json`; id `"<root>/<folder>"`), the
 *   User Library (`"user"`; the mock sees no user presets) and the factory presets;
 * - bpm / key parsed from file names (`parseNameMeta`), durations from the library metadata;
 * - every query filter ANDed, sorts with nulls last, `limit` clamped to `1..=200`;
 * - favourites and tags in memory; edits emit `IndexChanged`; `Rescan` emits
 *   `IndexProgress` then `IndexChanged` (synchronously);
 * - `Preview` plays `item.source` through `Media::Preview` (the mock ignores `sync`);
 * - `AddFolder` is native only: `Unsupported`; projects aren't indexed by the mock.
 * - `base-136`: `ImportFolder` adds an empty user folder (`Folder` root, unique name),
 *   `ImportFile` consumes a completed upload into it (an audio item at once: the mock's
 *   `Rescan` is synchronous anyway), `RenameFolder` renames it, `RemoveFolder` drops it
 *   with its items. Imported folders are index-only (no folder view, no lane import).
 */

import type {
  BrowserCommand,
  BrowserQuery,
  BrowserRoot,
  Event,
  LibraryItem,
  PresetDevice,
  ReplyValue,
} from "@/generated";
import { cmd } from "../../cmd";
import { fail } from "../documentReducer";
import { LIBRARY_FILES, LIBRARY_ID, wavSize } from "../library";
import type { MockHost } from "./host";
import type { UploadedAudio } from "./remote";

/** Top-level library folders the mock treats as sample packs. */
export const MOCK_PACKS: ReadonlyArray<string> = ["Vocals"];
export const USER_ROOT = "user";
export const FACTORY_ROOT = "factory";
export const PROJECTS_ROOT = "projects";
const LIBRARY_NAME = "Library";
const AUDIO_FILE = /\.(wav|wave|aif|aiff|aifc|flac|mp3|ogg|oga)$/i;
const MAX_LIMIT = 200;
/** Fake modification times: newest first in library order. */
const MODIFIED_BASE_MS = Date.UTC(2026, 0, 1);
const DAY_MS = 86_400_000;

const FACTORY_FILES = import.meta.glob<string>("../../../../../crates/ether-devices/presets/*/*.etherpreset", {
  query: "?raw",
  import: "default",
  eager: true,
});

/** Accidental-aware key token: `Am`, `C#maj`, `Bbminor`, `F#`. */
const KEY_TOKEN = /^([A-Ga-g])(#|b)?(m|min|minor|maj|major)?$/;

/** Engine rule: bpm and key from a file name (and its folder path, for the "loop" hint). */
export function parseNameMeta(name: string, path: string): { bpm: number | null; key: string | null } {
  const stem = name.replace(/\.[^.]+$/, "");
  const tokens = stem.split(/[\s_\-.()[\]]+/).filter(Boolean);
  let bpm: number | null = null;
  let key: string | null = null;
  let explicit = false;
  const bare: number[] = [];
  tokens.forEach((t, i) => {
    const m = /^(\d{2,3})bpm$/i.exec(t);
    if (m) {
      bpm = Number(m[1]);
      explicit = true;
      return;
    }
    if (/^bpm$/i.test(t) && i > 0 && /^\d{2,3}$/.test(tokens[i - 1]!)) {
      bpm = Number(tokens[i - 1]);
      explicit = true;
      return;
    }
    const k = KEY_TOKEN.exec(t);
    // A bare letter is too ambiguous ("Pad C" is a name); an accidental or quality is a key.
    if (k && (k[2] || k[3]) && key === null) {
      const minor = k[3] !== undefined && k[3].startsWith("m") && !k[3].startsWith("maj");
      key = `${k[1]!.toUpperCase()}${k[2] ?? ""} ${minor ? "minor" : "major"}`;
      explicit = true;
      return;
    }
    if (/^\d{2,3}$/.test(t)) bare.push(Number(t));
  });
  if (bpm === null) {
    const loopy = `${path}/${name}`.toLowerCase().includes("loop");
    const n = bare.find((v) => v >= 60 && v <= 200);
    if (n !== undefined && (loopy || explicit)) bpm = n;
  }
  return { bpm, key };
}

const baseName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

function libraryItems(): LibraryItem[] {
  return LIBRARY_FILES.filter((f) => f.kind === "Audio" || f.kind === "Midi").map((f, i) => {
    const name = baseName(f.path);
    const top = f.path.split("/")[0]!;
    const audio = f.kind === "Audio";
    const { bpm, key } = parseNameMeta(name, f.path);
    return {
      id: `${LIBRARY_ID}/${f.path}`,
      kind: audio ? "Audio" : "Midi",
      name,
      root: LIBRARY_ID,
      path: f.path,
      source: { type: "Location", location: { type: "Library", id: LIBRARY_ID }, path: f.path },
      preset: null,
      tags: [],
      favourite: false,
      meta: {
        duration_seconds: audio ? f.frames / f.sample_rate : null,
        sample_rate: audio ? f.sample_rate : null,
        channels: audio ? f.channels : null,
        bpm,
        key,
        pack: MOCK_PACKS.includes(top) ? top : LIBRARY_NAME,
        modified_ms: MODIFIED_BASE_MS - i * DAY_MS,
        size: audio ? wavSize(f.frames, f.channels) : 512,
      },
    };
  });
}

interface PresetFile {
  format?: string;
  preset?: { name?: string; device?: PresetDevice; meta?: { tags?: string[] } };
}

function factoryItems(): LibraryItem[] {
  return Object.entries(FACTORY_FILES).flatMap(([file, json]): LibraryItem[] => {
    const m = /presets\/([^/]+)\/([^/]+)\.etherpreset$/.exec(file);
    let f: PresetFile;
    try {
      f = JSON.parse(json) as PresetFile;
    } catch {
      return [];
    }
    if (!m || f.format !== "ethereal-preset" || !f.preset?.name) return [];
    const path = `${m[1]}/${m[2]}`;
    return [
      {
        id: `${FACTORY_ROOT}/${path}`,
        kind: "Preset",
        name: f.preset.name,
        root: FACTORY_ROOT,
        path,
        source: null,
        preset: { source: "Factory", id: path },
        tags: normalizeTags(f.preset.meta?.tags ?? []),
        favourite: false,
        meta: {
          duration_seconds: null,
          sample_rate: null,
          channels: null,
          bpm: null,
          key: null,
          pack: "Factory Presets",
          modified_ms: null,
          size: null,
        },
      },
    ];
  });
}

/** Tags as the engine stores them: trimmed, lowercase, no empties, deduplicated, sorted. */
export function normalizeTags(tags: ReadonlyArray<string>): string[] {
  return [...new Set(tags.map((t) => t.trim().toLowerCase()).filter((t) => t))].sort();
}

const byName = (a: LibraryItem, b: LibraryItem) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || a.id.localeCompare(b.id);

/** Nulls last, ascending (`desc` flips the non-null order only). */
function byNumber(get: (i: LibraryItem) => number | null | undefined, desc = false) {
  return (a: LibraryItem, b: LibraryItem) => {
    const x = get(a) ?? null;
    const y = get(b) ?? null;
    if (x === null || y === null) return x === y ? byName(a, b) : x === null ? 1 : -1;
    return (desc ? y - x : x - y) || byName(a, b);
  };
}

/** Relevance rank of an item for `text` (lower is better). */
function relevance(item: LibraryItem, text: string): number {
  const q = text.trim().toLowerCase();
  const name = item.name.toLowerCase();
  const stem = name.replace(/\.[^.]+$/, "");
  if (!q) return 4;
  if (name === q || stem === q) return 0;
  if (name.startsWith(q)) return 1;
  if (name.split(/[\s_\-.]+/).some((w) => w.startsWith(q))) return 2;
  if (name.includes(q)) return 3;
  return 4;
}

/** The runtime simulation (one per MockTransport). */
export class MockBrowser {
  private readonly items: LibraryItem[] = [...libraryItems(), ...factoryItems()];
  private readonly favourites = new Set<string>();
  /** User tags by item id (replacing the item's default tags). */
  private readonly tags = new Map<string, string[]>();
  /** `base-136`: imported folders (in creation order). */
  private readonly folders: BrowserRoot[] = [];
  private nextFolder = 1;

  constructor(
    private readonly host: MockHost | null,
    /** Consume a completed upload (`MockUploads.take`); `ImportFile` needs it. */
    private readonly takeUpload: ((upload: string) => UploadedAudio) | null = null,
  ) {}

  command(c: BrowserCommand): ReplyValue {
    switch (c.type) {
      case "ListRoots":
        return { type: "BrowserRoots", roots: this.roots() };
      case "Query":
        return { type: "BrowserPage", page: this.query(c.query) };
      case "SetFavourite":
        this.item(c.item);
        if (c.favourite) this.favourites.add(c.item);
        else this.favourites.delete(c.item);
        this.emit({ type: "Browser", event: { type: "IndexChanged" } });
        return { type: "Unit" };
      case "SetTags":
        this.item(c.item);
        this.tags.set(c.item, normalizeTags(c.tags));
        this.emit({ type: "Browser", event: { type: "IndexChanged" } });
        return { type: "Unit" };
      case "AddFolder":
        return fail("Unsupported", "adding folders needs the desktop app");
      case "RemoveFolder": {
        const folder = this.folders.findIndex((f) => f.id === c.root);
        if (folder >= 0) {
          this.folders.splice(folder, 1);
          this.dropItems((i) => i.root === c.root);
          this.emit({ type: "Browser", event: { type: "IndexChanged" } });
          return { type: "Unit" };
        }
        const root = this.roots().find((r) => r.id === c.root);
        if (!root) fail("NotFound", `root ${c.root}`);
        return fail("InvalidArgument", `${root.name} is not a user folder`);
      }
      case "ImportFolder": {
        const base = c.name
          .replace(/[/\\:\p{Cc}]/gu, " ")
          .trim()
          .replace(/^\.+/, "")
          .trim()
          .slice(0, 64)
          .trimEnd();
        if (!base) fail("InvalidArgument", `bad folder name ${JSON.stringify(c.name)}`);
        const taken = (n: string) => this.folders.some((f) => f.path?.toLowerCase() === `imported/${n}`.toLowerCase());
        let name = base;
        for (let n = 2; taken(name); n++) name = `${base} ${n}`;
        this.folders.push({ id: `folder-mock-${this.nextFolder++}`, name, kind: "Folder", path: `imported/${name}`, items: 0 });
        return { type: "BrowserRoots", roots: this.roots() };
      }
      case "ImportFile": {
        const folder = this.folders.find((f) => f.id === c.root);
        if (!this.takeUpload) fail("Unsupported", "uploads need the mock transport");
        // The upload is consumed either way.
        let audio: UploadedAudio | null = null;
        let error: unknown = null;
        try {
          audio = this.takeUpload(c.upload);
        } catch (e) {
          error = e;
        }
        if (!folder) fail("NotFound", `user folder ${c.root}`);
        const name = baseName(c.path);
        if (!c.path || c.path.split("/").some((seg) => !seg || seg === "." || seg === "..") || name.startsWith("."))
          fail("InvalidArgument", `bad file path ${JSON.stringify(c.path)}`);
        if (!AUDIO_FILE.test(name)) fail("InvalidArgument", `${name} is not an audio or MIDI file`);
        if (error) throw error;
        const a = audio!;
        const id = `${folder.id}/${c.path}`;
        const { bpm, key } = parseNameMeta(name, c.path);
        this.dropItems((i) => i.id === id);
        this.items.push({
          id,
          kind: "Audio",
          name,
          root: folder.id,
          path: c.path,
          source: { type: "Location", location: { type: "Library", id: folder.id }, path: c.path },
          preset: null,
          tags: [],
          favourite: false,
          meta: {
            duration_seconds: a.frames / a.sample_rate,
            sample_rate: a.sample_rate,
            channels: a.channels,
            bpm,
            key,
            pack: folder.name,
            modified_ms: MODIFIED_BASE_MS,
            size: wavSize(a.frames, a.channels),
          },
        });
        return { type: "Unit" };
      }
      case "RenameFolder": {
        const folder = this.folders.find((f) => f.id === c.root) ?? fail("NotFound", `user folder ${c.root}`);
        const name = c.name.trim();
        folder.name = name || folder.path!.slice("imported/".length);
        for (const i of this.items) if (i.root === folder.id) i.meta = { ...i.meta, pack: folder.name };
        this.emit({ type: "Browser", event: { type: "IndexChanged" } });
        return { type: "BrowserRoots", roots: this.roots() };
      }
      case "Rescan":
        this.rescan(c.root);
        return { type: "Unit" };
      case "Preview": {
        const item = this.item(c.item);
        if (item.kind !== "Audio" || !item.source) fail("InvalidArgument", `${item.name} can't be previewed`);
        if (!this.host) fail("Unsupported", "preview needs the mock host");
        this.host.execute(cmd("Media", { type: "Preview", source: item.source }));
        return { type: "Unit" };
      }
    }
  }

  private dropItems(pred: (i: LibraryItem) => boolean) {
    for (let at = this.items.length - 1; at >= 0; at--) if (pred(this.items[at]!)) this.items.splice(at, 1);
  }

  private emit(e: Event) {
    this.host?.emit(e);
  }

  private view(i: LibraryItem): LibraryItem {
    return { ...i, favourite: this.favourites.has(i.id), tags: this.tags.get(i.id) ?? i.tags };
  }

  private item(id: string): LibraryItem {
    const i = this.items.find((x) => x.id === id);
    if (!i) fail("NotFound", `library item ${id}`);
    return this.view(i);
  }

  private roots(): BrowserRoot[] {
    const count = (pred: (i: LibraryItem) => boolean) => this.items.filter(pred).length;
    return [
      { id: LIBRARY_ID, name: LIBRARY_NAME, kind: "Library", path: null, items: count((i) => i.root === LIBRARY_ID) },
      ...MOCK_PACKS.map(
        (p): BrowserRoot => ({
          id: `${LIBRARY_ID}/${p}`,
          name: p,
          kind: "Pack",
          path: null,
          items: count((i) => i.root === LIBRARY_ID && i.path.startsWith(`${p}/`)),
        }),
      ),
      { id: USER_ROOT, name: "User Library", kind: "Library", path: null, items: count((i) => i.root === USER_ROOT) },
      ...this.folders.map((f): BrowserRoot => ({ ...f, items: count((i) => i.root === f.id) })),
      { id: FACTORY_ROOT, name: "Factory Presets", kind: "Factory", path: null, items: count((i) => i.root === FACTORY_ROOT) },
    ];
  }

  private rescan(root: string | null) {
    const roots = this.roots().filter((r) => r.kind !== "Pack");
    const picked = root === null ? roots : this.roots().filter((r) => r.id === root);
    // Projects are indexed without being a listed root.
    if (root === PROJECTS_ROOT) picked.push({ id: PROJECTS_ROOT, name: "Projects", kind: "Library", path: null, items: 0 });
    if (picked.length === 0) fail("NotFound", `root ${root}`);
    for (const r of picked) this.emit({ type: "Browser", event: { type: "IndexProgress", root: r.id, scanned: r.items, total: r.items } });
    this.emit({ type: "Browser", event: { type: "IndexChanged" } });
  }

  private inRoot(i: LibraryItem, r: string): boolean {
    if (i.root === r) return true;
    const slash = r.indexOf("/");
    return slash > 0 && i.root === r.slice(0, slash) && i.path.startsWith(`${r.slice(slash + 1)}/`);
  }

  private matches(i: LibraryItem, q: BrowserQuery, words: string[]): boolean {
    if (q.kinds.length > 0 && !q.kinds.includes(i.kind)) return false;
    if (q.favourites_only && !i.favourite) return false;
    if (!q.tags.every((t) => i.tags.includes(t.trim().toLowerCase()))) return false;
    if (q.roots.length > 0 && !q.roots.some((r) => this.inRoot(i, r))) return false;
    if (q.folder) {
      const r0 = q.roots[0];
      const folder = q.folder.replace(/^\/+|\/+$/g, "");
      if (r0 !== undefined && i.root !== r0 && !this.inRoot(i, r0)) return false;
      if (!i.path.startsWith(`${folder}/`)) return false;
    }
    if (q.device && i.kind === "Preset" && !presetIsFor(i, q.device)) return false;
    if (words.length > 0) {
      const hay = [i.name, i.path, ...i.tags, i.meta.pack ?? "", i.meta.key ?? ""].join("\n").toLowerCase();
      if (!words.every((w) => hay.includes(w))) return false;
    }
    return true;
  }

  private query(q: BrowserQuery) {
    const words = q.text.toLowerCase().split(/\s+/).filter(Boolean);
    const found = this.items.map((i) => this.view(i)).filter((i) => this.matches(i, q, words));
    const sorters: Record<BrowserQuery["sort"], (a: LibraryItem, b: LibraryItem) => number> = {
      Name: byName,
      Recent: byNumber((i) => i.meta.modified_ms, true),
      Duration: byNumber((i) => i.meta.duration_seconds),
      Bpm: byNumber((i) => i.meta.bpm),
      Relevance: (a, b) => relevance(a, q.text) - relevance(b, q.text) || byName(a, b),
    };
    found.sort(sorters[q.sort]);
    const limit = Math.max(1, Math.min(MAX_LIMIT, q.limit));
    return { items: found.slice(q.offset, q.offset + limit), total: found.length, offset: q.offset };
  }
}

/** Factory presets live under `<device-key>/`, user presets under `Presets/<device-key>/`. */
function presetIsFor(i: LibraryItem, device: PresetDevice): boolean {
  if (device.type !== "Builtin") return false;
  const key = device.device.replace(/(?!^)([A-Z])/g, "-$1").toLowerCase();
  const path = i.root === USER_ROOT ? i.path.replace(/^Presets\//, "") : i.path;
  return path.startsWith(`${key}/`);
}
