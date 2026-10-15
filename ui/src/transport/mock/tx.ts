/**
 * A recording transaction over the mock's project: every write is applied immediately
 * (so later reads in the same command see it) and recorded twice, coalesced per entity:
 * - `changes()`: the final state of each touched entity → the `Patch` sent to the UI and
 *   the "redo" of the undo step;
 * - `inverse()`: the initial state of each touched entity → the "undo".
 *
 * Because patches are whole-entity snapshots, applying `inverse()` exactly restores the
 * pre-transaction state (history is linear), and `rollback()` makes commands atomic.
 */

import type { Device, Entity, EntityKey, PatchChange, Project, ProjectSettings } from "@/generated";
import { applyPatchChanges, entityKeyOf, getEntity, keyString, makeEntity, tableOf, type EntityType, type EntityValue } from "@/state/entities";

export class Tx {
  private readonly initial = new Map<string, PatchChange>();
  private readonly final = new Map<string, PatchChange>();

  constructor(readonly project: Project) {}

  /** Read an entity (sees this transaction's writes). */
  get<T extends EntityType>(type: T, id: string): EntityValue<T> | undefined {
    return tableOf(this.project, type)[id];
  }

  /** All rows of a table (sees this transaction's writes). */
  all<T extends EntityType>(type: T): EntityValue<T>[] {
    return Object.values(tableOf(this.project, type));
  }

  upsert<T extends EntityType>(type: T, value: EntityValue<T>): void {
    this.write({ type: "Upsert", entity: makeEntity(type, value) });
  }

  remove(type: EntityType, id: string): void {
    if (this.get(type, id) === undefined) return;
    this.write({ type: "Remove", key: { type, id } as EntityKey });
  }

  setSettings(settings: ProjectSettings): void {
    this.write({ type: "Settings", settings });
  }

  /** Apply an arbitrary change (used to replay history). */
  write(change: PatchChange): void {
    const k = changeKey(change);
    if (!this.initial.has(k)) this.initial.set(k, this.snapshot(change));
    applyPatchChanges(this.project, [change]);
    this.final.set(k, change);
  }

  get isEmpty(): boolean {
    return this.final.size === 0;
  }

  changes(): PatchChange[] {
    return [...this.final.values()];
  }

  inverse(): PatchChange[] {
    return [...this.initial.values()];
  }

  /**
   * `changes()` as the real controller sends them (perf-plugins, audit #4): a device whose
   * only difference is its params becomes one `DeviceParam` per changed param, and plugin
   * state blobs are stripped (`state: null`).
   */
  wireChanges(): PatchChange[] {
    const out: PatchChange[] = [];
    for (const [k, change] of this.final) {
      if (change.type !== "Upsert" || change.entity.type !== "Device") {
        out.push(change);
        continue;
      }
      const before = this.initial.get(k);
      const after = change.entity.value;
      if (before?.type === "Upsert" && before.entity.type === "Device" && sameButParams(before.entity.value, after)) {
        const params = new Set([...Object.keys(before.entity.value.params), ...Object.keys(after.params)].map(Number));
        for (const param of params) {
          const value = after.params[param] ?? null;
          if (value !== (before.entity.value.params[param] ?? null)) out.push({ type: "DeviceParam", id: after.id, param, value });
        }
        continue;
      }
      out.push(
        after.kind.type === "Plugin" && after.kind.plugin.state !== null ? { type: "Upsert", entity: { type: "Device", value: stripState(after) } } : change,
      );
    }
    return out;
  }

  /** Undo every write of this transaction. */
  rollback(): void {
    applyPatchChanges(this.project, this.inverse());
    this.initial.clear();
    this.final.clear();
  }

  /** The change that restores the current state of whatever `change` touches. */
  private snapshot(change: PatchChange): PatchChange {
    if (change.type === "Settings") return { type: "Settings", settings: this.project.settings };
    const key: EntityKey = change.type === "Upsert" ? entityKeyOf(change.entity) : change.type === "Remove" ? change.key : { type: "Device", id: change.id };
    const current: Entity | undefined = getEntity(this.project, key);
    return current ? { type: "Upsert", entity: current } : { type: "Remove", key };
  }
}

/** Coalescing key of a change: one per entity, plus one for the settings. */
export function changeKey(change: PatchChange): string {
  switch (change.type) {
    case "Settings":
      return "Settings";
    case "Upsert":
      return keyString(entityKeyOf(change.entity));
    case "Remove":
      return keyString(change.key);
    case "DeviceParam":
      return `DeviceParam:${change.id}:${change.param}`;
  }
}

function sameButParams(a: Device, b: Device): boolean {
  return JSON.stringify({ ...a, params: null }) === JSON.stringify({ ...b, params: null });
}

function stripState(d: Device): Device {
  return d.kind.type === "Plugin" ? { ...d, kind: { ...d.kind, plugin: { ...d.kind.plugin, state: null } } } : d;
}
