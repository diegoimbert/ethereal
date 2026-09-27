/**
 * Generic, domain-free access to the normalized `Project` tables by entity type, and the
 * patch application rule shared by the UI mirror and the MockTransport:
 *
 * - `Upsert` → `table[entity.value.id] = entity.value`
 * - `Remove` → `delete table[key.id]`
 * - `Settings` → `project.settings = settings`
 *
 * `applyPatchChanges` mutates the given project in place, so it works both on plain
 * objects and on immer drafts.
 */

import type { Entity, EntityKey, PatchChange, Project } from "@/generated";

export type EntityType = Entity["type"];

/** The value type stored for an entity type (e.g. `EntityValue<"Track">` = `Track`). */
export type EntityValue<T extends EntityType> = Extract<Entity, { type: T }>["value"];

/** The `Project` field holding each entity table. */
export const TABLE_OF = {
  Track: "tracks",
  Clip: "clips",
  Note: "notes",
  Device: "devices",
  Send: "sends",
  AutomationLane: "automation_lanes",
  AutomationPoint: "automation_points",
  TempoPoint: "tempo_points",
  TimeSignature: "time_signatures",
  WarpMarker: "warp_markers",
  Media: "media",
  // Roadmap v2. Pads come after their rack devices; pad-chain devices share the Device
  // table (a full dump that must be strictly ordered should emit rack devices, pads, then
  // pad devices, like Rust `Project::entities`).
  DrumPad: "drum_pads",
  Marker: "markers",
  MidiMapping: "midi_mappings",
  // v0.2 (contracts-3). Take clips share the Clip table (`Clip.lane`), rack-chain devices the
  // Device table (`Device.chain`); lanes come before their clips and regions, chains after
  // their rack and before their devices, modulators and mappings after their devices.
  TakeLane: "take_lanes",
  CompRegion: "comp_regions",
  RackChain: "rack_chains",
  Modulator: "modulators",
  ModMapping: "mod_mappings",
} as const satisfies Record<EntityType, keyof Project>;

/** Every entity type, parents before children (useful for ordered full-state dumps). */
export const ENTITY_TYPES: ReadonlyArray<EntityType> = Object.keys(TABLE_OF) as EntityType[];

/** The table of `type` in `project`, typed by entity type. */
export function tableOf<T extends EntityType>(project: Project, type: T): Record<string, EntityValue<T>> {
  // Each TABLE_OF entry points at the table whose value type is EntityValue<T>; TS can't
  // correlate the two unions, hence the cast (contained here).
  return project[TABLE_OF[type]] as unknown as Record<string, EntityValue<T>>;
}

export function entityKeyOf(entity: Entity): EntityKey {
  return { type: entity.type, id: entity.value.id } as EntityKey;
}

/** Stable string form of an entity key (`"Track:01H…"`), for maps/sets. */
export function keyString(key: EntityKey): string {
  return `${key.type}:${key.id}`;
}

export function makeEntity<T extends EntityType>(type: T, value: EntityValue<T>): Entity {
  return { type, value } as Entity;
}

export function getEntity(project: Project, key: EntityKey): Entity | undefined {
  const value = tableOf(project, key.type)[key.id];
  return value === undefined ? undefined : makeEntity(key.type, value);
}

/** Apply patch changes to `project` in place (plain object or immer draft). */
export function applyPatchChanges(project: Project, changes: ReadonlyArray<PatchChange>): void {
  for (const change of changes) {
    switch (change.type) {
      case "Upsert": {
        const { type, value } = change.entity;
        tableOf(project, type)[value.id] = value;
        break;
      }
      case "Remove":
        delete tableOf(project, change.key.type)[change.key.id];
        break;
      case "Settings":
        project.settings = change.settings;
        break;
    }
  }
}
