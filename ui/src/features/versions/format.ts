// Pure helpers for the versions dialog and the recovery dialog (tested in format.test.ts).
import type { TableDiff, VersionDiff, VersionInfo, VersionKind } from "@/generated";
import { formatDate, formatDateTime, t, type MessageKey } from "@/i18n";

const KIND_LABEL: Record<VersionKind, MessageKey> = {
  Autosave: "versions.kind.autosave",
  Manual: "versions.kind.manual",
  BeforeRestore: "versions.kind.before_restore",
};

/** The row title: the version's name, else what made it. */
export function versionTitle(v: Pick<VersionInfo, "name" | "kind">): string {
  return v.name ?? kindLabel(v.kind);
}

export function kindLabel(kind: VersionKind): string {
  return t(KIND_LABEL[kind]);
}

/** "just now", "12 min ago", "14:05" (today), "yesterday 09:12", else a date and time. */
export function formatWhen(ms: number, now: number = Date.now()): string {
  const diff = now - ms;
  if (diff >= 0 && diff < 60_000) return t("project.time.just_now");
  if (diff >= 0 && diff < 3_600_000) return t("project.time.minutes_ago", { count: Math.floor(diff / 60_000) });
  const d = new Date(ms);
  const time = formatDate(d, { hour: "2-digit", minute: "2-digit" });
  const today = new Date(now);
  if (today.toDateString() === d.toDateString()) return time;
  const yesterday = new Date(now - 86_400_000);
  if (yesterday.toDateString() === d.toDateString()) return t("versions.when.yesterday", { time });
  const thisYear = d.getFullYear() === today.getFullYear();
  return `${formatDate(d, thisYear ? { month: "short", day: "numeric" } : { month: "short", day: "numeric", year: "numeric" })} ${time}`;
}

/** Full date and time (tooltips). */
export function formatExact(ms: number): string {
  return formatDateTime(ms, { dateStyle: "medium", timeStyle: "medium" });
}

/** "1.2 mb", "340 kb". */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} b`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} kb`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} mb`;
}

/** The engine's diff tables (`ether_model::template::entity_table`), by name. */
const TABLE_LABEL: Record<string, MessageKey> = {
  tracks: "versions.table.tracks",
  clips: "versions.table.clips",
  notes: "versions.table.notes",
  devices: "versions.table.devices",
  sends: "versions.table.sends",
  automation_lanes: "versions.table.automation_lanes",
  automation_points: "versions.table.automation_points",
  tempo_points: "versions.table.tempo_points",
  time_signatures: "versions.table.time_signatures",
  warp_markers: "versions.table.warp_markers",
  media: "versions.table.media",
  markers: "versions.table.markers",
  midi_mappings: "versions.table.midi_mappings",
  drum_pads: "versions.table.drum_pads",
  take_lanes: "versions.table.take_lanes",
  comp_regions: "versions.table.comp_regions",
  rack_chains: "versions.table.rack_chains",
  modulators: "versions.table.modulators",
  mod_mappings: "versions.table.mod_mappings",
  chat: "versions.table.chat",
  pinned_notes: "versions.table.pinned_notes",
  expression_lanes: "versions.table.expression_lanes",
  note_expressions: "versions.table.note_expressions",
};

/** `automation_points` → "automation points" (a table this build doesn't know: its name, spaced). */
export function tableLabel(table: string): string {
  const key = Object.hasOwn(TABLE_LABEL, table) ? TABLE_LABEL[table] : undefined;
  return key ? t(key) : table.replace(/_/g, " ");
}

/** "+1 −2 ~3" (zero counts left out). */
export function countsLabel(t: Pick<TableDiff, "added" | "removed" | "changed">): string {
  const parts: string[] = [];
  if (t.added) parts.push(`+${t.added}`);
  if (t.removed) parts.push(`−${t.removed}`);
  if (t.changed) parts.push(`~${t.changed}`);
  return parts.join(" ");
}

/** Tables in reading order: the ones people think in first. */
const ORDER = ["tracks", "clips", "notes", "devices", "automation_lanes", "automation_points", "markers", "media"];

export function sortTables(diff: VersionDiff): TableDiff[] {
  const rank = (t: string) => {
    const i = ORDER.indexOf(t);
    return i < 0 ? ORDER.length : i;
  };
  return [...diff.tables].sort((a, b) => rank(a.table) - rank(b.table) || a.table.localeCompare(b.table));
}

export function isEmptyDiff(diff: VersionDiff): boolean {
  return diff.tables.length === 0 && !diff.settings_changed;
}
