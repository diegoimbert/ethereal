/**
 * Mock of `Version::*` (v0.3, `project-versions`; CONTRACTS.md §13.11): rolling autosaved
 * versions of the open project (`List`, `Create`, `Restore`, `Delete`, `Rename`, `Compare`)
 * and crash recovery (`ListRecoverable`, `Recover`, `DiscardRecovery`) next to the mock
 * project store. Mirrors `crates/ether-controller/src/versions/`:
 *
 * - an autosave version when the document changed since the last one, at most every
 *   `VERSION_INTERVAL_MS`, keeping the newest `MAX_AUTOSAVE_VERSIONS`;
 * - `Restore` snapshots a `BeforeRestore` version first, then replaces the document (dirty);
 * - the mock never crashes, so tests and demos leave a session marker behind with
 *   `MockVersions.simulateCrash(project)`; `ListRecoverable` then offers it when its newest
 *   version is newer than the saved document and differs from it.
 */

import type { Event, Project, ProjectId, ProjectSummary, ReplyValue, TableDiff, VersionCommand, VersionDiff, VersionInfo, VersionKind } from "@/generated";
import { fail } from "../documentReducer";

/** `VERSION_INTERVAL_MS` / `MAX_AUTOSAVE_VERSIONS` (ether_protocol::versions). */
export const VERSION_INTERVAL_MS = 5 * 60_000;
export const MAX_AUTOSAVE_VERSIONS = 50;
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
  return n ? n.slice(0, 120) : null;
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
    if (this.host.revision() === this.lastRevision || now - this.lastVersionAt < this.intervalMs) return;
    this.lastRevision = this.host.revision();
    this.lastVersionAt = now;
    this.write("Autosave", null);
    const list = this.list(this.host.project().id);
    const old = new Set(
      list
        .filter((v) => v.info.kind === "Autosave")
        .slice(MAX_AUTOSAVE_VERSIONS)
        .map((v) => v.info.id),
    );
    if (old.size) this.byProject.set(this.host.project().id, list.filter((v) => !old.has(v.info.id)));
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
      // base-131: the mock's session markers are the simulated crashes.
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

  private changed(): void {
    this.host.emit({ type: "Version", event: { type: "Changed" } });
  }
}
