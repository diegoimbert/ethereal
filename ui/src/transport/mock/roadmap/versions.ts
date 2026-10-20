/**
 * Mock of `Version::*` (v0.3; CONTRACTS.md §13.11): rolling autosaved
 * versions of the open project (`List`, `Create`, `Restore`, `Delete`, `Rename`, `Compare`)
 * and crash recovery (`ListRecoverable`, `Recover`, `DiscardRecovery`) next to the mock
 * project store. Mirrors `crates/ether-controller/src/versions/`:
 *
 * - an autosave version when the document changed since the last one, at most every
 *   `VERSION_INTERVAL_MS`; unnamed autosave versions thinned by `retainAutosaves` (the
 *   controller's `versions::retain`: 5 recent, 5 hourly, 20 spread logarithmically);
 * - `Restore` snapshots a `BeforeRestore` version first (the newest
 *   `MAX_BEFORE_RESTORE_VERSIONS` unnamed ones kept), then replaces the document (dirty);
 * - named versions are never pruned;
 * - the mock never crashes, so tests and demos leave a session marker behind with
 *   `MockVersions.simulateCrash(project)`; `ListRecoverable` then offers it when its newest
 *   version is newer than the saved document and differs from it.
 */

import type { Event, Project, ProjectId, ProjectSummary, ReplyValue, TableDiff, VersionCommand, VersionDiff, VersionInfo, VersionKind } from "@/generated";
import { fail } from "../documentReducer";
import { takeChars } from "./shared";

/** The retention constants of `ether_protocol::versions`. */
export const VERSION_INTERVAL_MS = 60_000;
export const RECENT_AUTOSAVE_VERSIONS = 5;
export const HOURLY_AUTOSAVE_VERSIONS = 5;
export const SPREAD_AUTOSAVE_VERSIONS = 20;
export const MAX_AUTOSAVE_VERSIONS = RECENT_AUTOSAVE_VERSIONS + HOURLY_AUTOSAVE_VERSIONS + SPREAD_AUTOSAVE_VERSIONS;
const HOUR_MS = 3_600_000;
const SPREAD_MIN_AGE_MS = 6 * HOUR_MS;
const SPREAD_MAX_AGE_MS = 365 * 24 * HOUR_MS;
export const MAX_BEFORE_RESTORE_VERSIONS = 10;

interface Stamp {
  t: number;
  created: number;
  id: string;
}

/** Newest first; among equal effective times, the one really written then first. */
const newestFirst = (a: Stamp, b: Stamp) => b.t - a.t || a.created - b.created || (b.id < a.id ? -1 : b.id > a.id ? 1 : 0);
const older = (a: Stamp, b: Stamp) => (newestFirst(a, b) > 0 ? a : b);

const SPREAD_RATIO = Math.pow(SPREAD_MAX_AGE_MS / SPREAD_MIN_AGE_MS, 1 / SPREAD_AUTOSAVE_VERSIONS);
function spreadBucket(age: number): number {
  let i = 0;
  while (i < SPREAD_AUTOSAVE_VERSIONS - 1 && age >= Math.floor(SPREAD_MIN_AGE_MS * Math.pow(SPREAD_RATIO, i + 1))) i++;
  return i;
}

/**
 * The autosave versions to keep at `now` (mirrors `versions::retain`, CONTRACTS.md §13.11):
 * recent (newest, then each ≥ 1 min older, up to 5), hourly (the oldest of each clock hour,
 * 5 newest hours), spread (the oldest per geometric age bucket, 6 h to 1 year, 20), then
 * fill up to 30 by farthest log-age.
 */
export function retainAutosaves(versions: ReadonlyArray<VersionInfo>, now: number): Set<string> {
  const all: Stamp[] = versions.map((v) => ({ t: Math.min(v.created_ms, now), created: v.created_ms, id: v.id })).sort(newestFirst);
  const kept = new Set<string>();
  let last: number | null = null;
  for (const s of all) {
    if (kept.size === RECENT_AUTOSAVE_VERSIONS) break;
    if (last === null || last - s.t >= VERSION_INTERVAL_MS) {
      kept.add(s.id);
      last = s.t;
    }
  }
  const hours = new Map<number, Stamp>();
  for (const s of all) {
    if (kept.has(s.id)) continue;
    const h = Math.floor(s.t / HOUR_MS);
    const o = hours.get(h);
    hours.set(h, o ? older(o, s) : s);
  }
  for (const h of [...hours.keys()].sort((a, b) => b - a).slice(0, HOURLY_AUTOSAVE_VERSIONS)) kept.add(hours.get(h)!.id);
  const buckets = new Map<number, Stamp>();
  for (const s of all) {
    if (kept.has(s.id)) continue;
    const b = spreadBucket(now - s.t);
    const o = buckets.get(b);
    buckets.set(b, o ? older(o, s) : s);
  }
  for (const s of buckets.values()) kept.add(s.id);
  const logAge = (s: Stamp) => Math.log(now - s.t + VERSION_INTERVAL_MS);
  const keptLogs = all.filter((s) => kept.has(s.id)).map(logAge);
  const target = Math.min(MAX_AUTOSAVE_VERSIONS, all.length);
  while (kept.size < target) {
    let best: { d: number; s: Stamp } | null = null;
    for (let i = all.length - 1; i >= 0; i--) {
      const s = all[i]!;
      if (kept.has(s.id)) continue;
      const l = logAge(s);
      const d = keptLogs.reduce((m, k) => Math.min(m, Math.abs(k - l)), Infinity);
      if (!best || d > best.d) best = { d, s };
    }
    if (!best) break;
    kept.add(best.s.id);
    keptLogs.push(logAge(best.s));
  }
  return kept;
}
/** Names listed per table in a diff, at most. */
const MAX_NAMES = 50;
const NAMED_TABLES = new Set(["tracks", "clips"]);

const KIND_TAG: Record<VersionKind, string> = { Autosave: "autosave", Manual: "manual", BeforeRestore: "before-restore" };

interface StoredVersion {
  info: VersionInfo;
  /** JSON of the document (what the `.ether` file holds). */
  json: string;
}

export interface VersionsHost {
  /** The open document. */
  project(): Project;
  /** Document revision (bumps on every edit). */
  revision(): number;
  /** Wall clock (ms). */
  now(): number;
  emit(event: Event): void;
  /** Stored projects with their last save time. */
  summaries(): ProjectSummary[];
  /** The saved document of a stored project, if any. */
  savedJson(id: ProjectId): string | undefined;
  /** Make `project` the open document and mark it dirty (restore, recover). */
  replaceDocument(project: Project): void;
  /** Save the open project if dirty (before switching to a recovered one). */
  saveIfDirty(): void;
}

/** What changed from `base` to `target` (per table, plus settings). */
export function diffProjects(base: Project, target: Project): VersionDiff {
  const b = base as unknown as Record<string, unknown>;
  const t = target as unknown as Record<string, unknown>;
  const keys = [...new Set([...Object.keys(b), ...Object.keys(t)])].sort();
  const tables: TableDiff[] = [];
  for (const key of keys) {
    if (key === "id" || key === "settings") continue;
    const bm = (b[key] ?? {}) as Record<string, unknown>;
    const tm = (t[key] ?? {}) as Record<string, unknown>;
    if (typeof bm !== "object" || typeof tm !== "object") continue;
    const row: TableDiff = { table: key, added: 0, removed: 0, changed: 0, names: [] };
    const note = (v: unknown) => {
      const name = (v as { name?: unknown } | null)?.name;
      if (NAMED_TABLES.has(key) && row.names.length < MAX_NAMES && typeof name === "string" && name.trim() && !row.names.includes(name)) {
        row.names.push(name);
      }
    };
    for (const [id, v] of Object.entries(tm)) {
      if (!(id in bm)) {
        row.added++;
        note(v);
      } else if (JSON.stringify(bm[id]) !== JSON.stringify(v)) {
        row.changed++;
        note(v);
      }
    }
    for (const [id, v] of Object.entries(bm)) {
      if (!(id in tm)) {
        row.removed++;
        note(v);
      }
    }
    if (row.added + row.removed + row.changed > 0) tables.push(row);
  }
  return { tables, settings_changed: JSON.stringify(b.settings) !== JSON.stringify(t.settings) };
}

const cleanName = (name: string | null): string | null => {
  const n = name?.trim();
  return n ? takeChars(n, 120) : null;
};

export class MockVersions {
  private readonly byProject = new Map<ProjectId, StoredVersion[]>();
  /** Projects whose session marker survived (a crash). */
  private readonly markers = new Set<ProjectId>();
  private session: ProjectId | null = null;
  private lastRevision = 0;
  private lastVersionAt = 0;
  /** Minimum time between autosave versions (tests shorten it; the mock ticks in 16 ms steps). */
  intervalMs = VERSION_INTERVAL_MS;

  constructor(
    private readonly host: VersionsHost,
    private readonly parse: (json: string) => Project,
    private readonly serialize: (project: Project) => string,
  ) {}

  /** A project became the open document (its session starts; the previous one closed). */
  projectLoaded(): void {
    const id = this.host.project().id;
    this.lastRevision = this.host.revision();
    this.lastVersionAt = this.host.now();
    this.session = id;
  }

  /** Leave `project`'s session marker behind, as a killed app would (tests, demos). */
  simulateCrash(project: ProjectId = this.host.project().id): void {
    this.markers.add(project);
  }

  /** Called on every mock step: a rolling autosave version when due. */
  step(): void {
    const now = this.host.now();
    if (this.session === null) {
      // The initial project (loaded before any step).
      this.session = this.host.project().id;
      this.lastVersionAt = now;
    }
    // A clock gone backwards makes the next version due.
    if (this.host.revision() === this.lastRevision || (now >= this.lastVersionAt && now - this.lastVersionAt < this.intervalMs)) return;
    this.lastRevision = this.host.revision();
    this.lastVersionAt = now;
    this.write("Autosave", null);
    this.prune(now);
    this.changed();
  }

  command(c: VersionCommand): ReplyValue {
    switch (c.type) {
      case "List":
        return { type: "Versions", versions: this.list(this.host.project().id).map((v) => ({ ...v.info })) };
      case "Create": {
        const version = this.write("Manual", cleanName(c.name));
        this.changed();
        return { type: "Version", version: { ...version } };
      }
      case "Restore": {
        const v = this.find(c.version);
        const project = { ...this.parse(v.json), id: this.host.project().id };
        this.write("BeforeRestore", null);
        this.prune(this.host.now());
        this.host.replaceDocument(project);
        this.projectLoaded();
        this.changed();
        return { type: "Unit" };
      }
      case "Delete": {
        const id = this.host.project().id;
        this.find(c.version);
        this.byProject.set(
          id,
          this.list(id).filter((v) => v.info.id !== c.version),
        );
        this.changed();
        return { type: "Unit" };
      }
      case "Rename":
        this.find(c.version).info.name = cleanName(c.name);
        this.changed();
        return { type: "Unit" };
      case "Compare": {
        const base = this.parse(this.find(c.version).json);
        const target = c.against === null ? this.host.project() : this.parse(this.find(c.against).json);
        return { type: "VersionDiff", diff: diffProjects({ ...base, id: target.id }, target) };
      }
      case "ListRecoverable":
        return { type: "Recoverable", projects: this.recoverable() };
      case "Recover": {
        const newest = this.list(c.project)[0] ?? fail("InvalidArgument", `project ${c.project} has no versions to recover`);
        const project = { ...this.parse(newest.json), id: c.project };
        if (project.id !== this.host.project().id) this.host.saveIfDirty();
        this.markers.delete(c.project);
        this.host.replaceDocument(project);
        this.projectLoaded();
        return { type: "Project", project };
      }
      case "DiscardRecovery":
        this.markers.delete(c.project);
        return { type: "Unit" };
      // The mock's session markers are the simulated crashes.
      case "SessionStatus":
        return { type: "SessionStatus", unclean: [...this.markers] };
    }
  }

  private recoverable() {
    const out = [];
    for (const s of this.host.summaries()) {
      if (!this.markers.has(s.id)) continue;
      const newest = this.list(s.id)[0];
      if (!newest || newest.info.created_ms <= s.modified_ms) continue;
      const saved = this.host.savedJson(s.id);
      if (saved !== undefined) {
        const d = diffProjects({ ...this.parse(saved), id: s.id }, { ...this.parse(newest.json), id: s.id });
        if (!d.tables.length && !d.settings_changed) continue;
      }
      out.push({ project: s.id, name: s.name, version: { ...newest.info }, saved_ms: s.modified_ms });
    }
    return out;
  }

  private list(id: ProjectId): StoredVersion[] {
    const list = this.byProject.get(id) ?? [];
    return [...list].sort((a, b) => b.info.created_ms - a.info.created_ms || b.info.id.localeCompare(a.info.id));
  }

  private find(version: string): StoredVersion {
    return this.list(this.host.project().id).find((v) => v.info.id === version) ?? fail("NotFound", `version ${version}`);
  }

  private write(kind: VersionKind, name: string | null): VersionInfo {
    const project = this.host.project();
    const list = this.byProject.get(project.id) ?? [];
    let created = Math.floor(this.host.now());
    const idOf = (ms: number) => `${ms}-${KIND_TAG[kind]}`;
    while (list.some((v) => v.info.id === idOf(created))) created++;
    const json = this.serialize(project);
    const info: VersionInfo = { id: idOf(created), kind, name, created_ms: created, size: json.length };
    list.push({ info, json });
    this.byProject.set(project.id, list);
    return info;
  }

  /** `versions::prune_versions`: named versions are never pruned. */
  private prune(now: number): void {
    const id = this.host.project().id;
    const list = this.list(id);
    const unnamed = (kind: VersionKind) => list.filter((v) => v.info.kind === kind && v.info.name === null);
    const keep = retainAutosaves(
      unnamed("Autosave").map((v) => v.info),
      now,
    );
    const doomed = new Set([
      ...unnamed("Autosave")
        .filter((v) => !keep.has(v.info.id))
        .map((v) => v.info.id),
      ...unnamed("BeforeRestore")
        .slice(MAX_BEFORE_RESTORE_VERSIONS)
        .map((v) => v.info.id),
    ]);
    if (doomed.size)
      this.byProject.set(
        id,
        list.filter((v) => !doomed.has(v.info.id)),
      );
  }

  private changed(): void {
    this.host.emit({ type: "Version", event: { type: "Changed" } });
  }
}
