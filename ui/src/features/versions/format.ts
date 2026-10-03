// Pure helpers for the versions dialog and the recovery dialog (tested in format.test.ts).
import type { TableDiff, VersionDiff, VersionInfo, VersionKind } from "@/generated";

const KIND_LABEL: Record<VersionKind, string> = {
  Autosave: "Autosave",
  Manual: "Saved version",
  BeforeRestore: "Before restore",
};

/** The row title: the version's name, else what made it. */
export function versionTitle(v: Pick<VersionInfo, "name" | "kind">): string {
  return v.name ?? KIND_LABEL[v.kind];
}

export function kindLabel(kind: VersionKind): string {
  return KIND_LABEL[kind];
}

/** "just now", "12 min ago", "14:05" (today), "Yesterday 09:12", else a date and time. */
export function formatWhen(ms: number, now: number = Date.now()): string {
  const diff = now - ms;
  if (diff >= 0 && diff < 60_000) return "just now";
  if (diff >= 0 && diff < 3_600_000) return `${Math.floor(diff / 60_000)} min ago`;
  const d = new Date(ms);
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const today = new Date(now);
  if (today.toDateString() === d.toDateString()) return time;
  const yesterday = new Date(now - 86_400_000);
  if (yesterday.toDateString() === d.toDateString()) return `Yesterday ${time}`;
  return `${d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: d.getFullYear() === today.getFullYear() ? undefined : "numeric" })} ${time}`;
}

/** Full date and time (tooltips). */
export function formatExact(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "medium" });
}

/** "1.2 MB", "340 KB". */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** `automation_points` → "Automation points". */
export function tableLabel(table: string): string {
  const words = table.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
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
