/**
 * The UI mirror's note table, sharded: `project.notes` keeps its type (a read-only
 * `Record<NoteId, Note>`), but the store holds it as a proxy over 256 frozen buckets keyed
 * by a hash of the note id. A patch copies only the buckets it touches: a one-note edit in
 * a 262k-note project copies ~1k entries instead of the whole table (V8 copies a 262k-key
 * object in ~100 ms). Reads work as on a frozen plain object: `notes[id]`, `id in notes`,
 * `Object.keys/values/entries`, spread, `JSON.stringify`; writes are refused.
 *
 * Only the store makes these (`loadProject`, patches); the mock engine's own working copy
 * stays a plain object.
 */

import type { Note } from "@/generated";

type Notes = Record<string, Note>;

const BUCKETS = 256;

/** FNV-1a over the id's last 8 characters (a ULID's random part). */
function bucketOf(id: string): number {
  let h = 0x811c9dc5;
  for (let i = Math.max(0, id.length - 8); i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0) & (BUCKETS - 1);
}

interface Table {
  readonly buckets: ReadonlyArray<Readonly<Notes>>;
  /** Every id, built on the first full iteration. */
  keys: string[] | null;
}

const TABLES = new WeakMap<object, Table>();

/** What `Object.getPrototypeOf(table)` says: not a plain object (immer leaves it alone). */
class NoteTable {}
const PROTO = NoteTable.prototype;

function lookup(t: Table, key: string | symbol): Note | undefined {
  if (typeof key !== "string") return undefined;
  const b = t.buckets[bucketOf(key)]!;
  return Object.hasOwn(b, key) ? b[key] : undefined;
}

function allKeys(t: Table): string[] {
  if (!t.keys) {
    const keys: string[] = [];
    for (const b of t.buckets) for (const k in b) keys.push(k);
    t.keys = keys;
  }
  return t.keys;
}

const HANDLER: ProxyHandler<object> = {
  get(target, key) {
    const t = TABLES.get(target)!;
    const v = lookup(t, key);
    if (v !== undefined) return v;
    // `Object.prototype` members (`toString`, `hasOwnProperty`...) as on a plain object.
    return typeof key === "string" && key in Object.prototype ? (Object.prototype as Record<string, unknown>)[key] : undefined;
  },
  has(target, key) {
    return lookup(TABLES.get(target)!, key) !== undefined;
  },
  ownKeys(target) {
    return allKeys(TABLES.get(target)!);
  },
  getOwnPropertyDescriptor(target, key) {
    const v = lookup(TABLES.get(target)!, key);
    return v === undefined ? undefined : { value: v, writable: false, enumerable: true, configurable: true };
  },
  getPrototypeOf() {
    return PROTO;
  },
  set() {
    return false;
  },
  defineProperty() {
    return false;
  },
  deleteProperty() {
    return false;
  },
  setPrototypeOf() {
    return false;
  },
  preventExtensions() {
    return false;
  },
};

function make(t: Table): Notes {
  const target = {};
  TABLES.set(target, t);
  const proxy = new Proxy(target, HANDLER) as Notes;
  PROXIES.set(proxy, target);
  return proxy;
}

const PROXIES = new WeakMap<object, object>();

/** A sharded note table: `notes` is a store table already, or any plain table (copied). */
export function isNoteTable(x: unknown): boolean {
  return typeof x === "object" && x !== null && PROXIES.has(x);
}

export function noteTable(notes: Readonly<Notes>): Notes {
  if (isNoteTable(notes)) return notes as Notes;
  const buckets: Notes[] = Array.from({ length: BUCKETS }, () => ({}));
  for (const k in notes) buckets[bucketOf(k)]![k] = Object.freeze(notes[k]!);
  return make({ buckets: buckets.map((b) => Object.freeze(b)), keys: null });
}

/**
 * `table` with `ops` applied in order (`[id, note]` upserts, `[id, undefined]` removes),
 * sharing the untouched buckets.
 */
export function withNotes(table: Readonly<Notes>, ops: ReadonlyArray<readonly [string, Note | undefined]>): Notes {
  const t = TABLES.get(PROXIES.get(noteTable(table))!)!;
  const buckets = t.buckets.slice() as Notes[];
  const copied = new Set<number>();
  for (const [id, note] of ops) {
    const b = bucketOf(id);
    if (!copied.has(b)) {
      copied.add(b);
      buckets[b] = { ...buckets[b] };
    }
    if (note === undefined) delete buckets[b]![id];
    else buckets[b]![id] = Object.freeze(note);
  }
  for (const b of copied) Object.freeze(buckets[b]);
  return make({ buckets, keys: null });
}

/** The notes of a store table without going through the proxy (every bucket's values). */
export function noteValues(table: object): Note[] | null {
  const target = PROXIES.get(table);
  if (!target) return null;
  const out: Note[] = [];
  for (const b of TABLES.get(target)!.buckets) for (const k in b) out.push(b[k]!);
  return out;
}
