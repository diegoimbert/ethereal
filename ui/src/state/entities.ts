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
  // v0.2. Take clips share the Clip table (`Clip.lane`), rack-chain devices the
  // Device table (`Device.chain`); lanes come before their clips and regions, chains after
  // their rack and before their devices, modulators and mappings after their devices.
  TakeLane: "take_lanes",
  CompRegion: "comp_regions",
  RackChain: "rack_chains",
  Modulator: "modulators",
  ModMapping: "mod_mappings",
  // Chat journal and pinned notes (no parents).
  ChatMessage: "chat",
  PinnedNote: "pinned_notes",
  // v0.3: expression lanes after their clips, note expressions after their notes.
  ExpressionLane: "expression_lanes",
  NoteExpression: "note_expressions",
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
      case "DeviceParam": {
        // One param of an existing device (ignored if unknown);
        // the device's other fields keep their identity (immer).
        const device = project.devices[change.id];
        if (!device) break;
        if (change.value === null || change.value === undefined) delete device.params[change.param];
        else device.params[change.param] = change.value;
        break;
      }
    }
  }
}

function deepFreeze<T>(v: T): T {
  if (v !== null && typeof v === "object" && !Object.isFrozen(v)) {
    Object.freeze(v);
    for (const x of Object.values(v as object)) deepFreeze(x);
  }
  return v;
}

/**
 * `changes` applied to an immutable (frozen) `project` without immer: the root and each
 * table the patch touches are copied once (immer's draft of a big table costs its copy plus
 * a scan of every entry when finalizing), untouched tables and entities keep their identity,
 * and everything new is frozen like immer would. The result equals
 * `applyPatchChanges` on a copy.
 */
export function withPatchChanges(project: Project, changes: ReadonlyArray<PatchChange>): Project {
  const next = { ...project } as Project;
  const copied = new Set<keyof Project>();
  const table = (type: EntityType): Record<string, unknown> => {
    const name = TABLE_OF[type];
    if (!copied.has(name)) {
      copied.add(name);
      (next as unknown as Record<string, unknown>)[name] = { ...(next[name] as object) };
    }
    return next[name] as unknown as Record<string, unknown>;
  };
  const devices = new Set<string>();
  for (const change of changes) {
    switch (change.type) {
      case "Upsert": {
        const { type, value } = change.entity;
        table(type)[value.id] = deepFreeze(value);
        if (type === "Device") devices.delete(value.id);
        break;
      }
      case "Remove":
        delete table(change.key.type)[change.key.id];
        break;
      case "Settings":
        next.settings = deepFreeze(change.settings);
        break;
      case "DeviceParam": {
        const t = table("Device") as Project["devices"];
        const device = t[change.id];
        if (!device) break;
        // One copy of the device (and its params) per patch, however many params change.
        const d = devices.has(change.id) ? device : { ...device, params: { ...device.params } };
        devices.add(change.id);
        if (change.value === null || change.value === undefined) delete d.params[change.param];
        else d.params[change.param] = change.value;
        t[change.id] = d;
        break;
      }
    }
  }
  for (const id of devices) {
    const d = next.devices[id];
    if (d) deepFreeze(d);
  }
  for (const name of copied) Object.freeze(next[name]);
  return Object.freeze(next);
}
