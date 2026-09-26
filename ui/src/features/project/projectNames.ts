// Pure helpers for the project manager (tested in projectNames.test.ts).
import type { ProjectSummary } from "@/generated";

/** Most recently saved first, then by name. */
export function sortProjects(projects: ReadonlyArray<ProjectSummary>): ProjectSummary[] {
  return [...projects].sort((a, b) => b.modified_ms - a.modified_ms || a.name.localeCompare(b.name));
}

/**
 * `base`, or `base 2`, `base 3`, ... : the first name not used by `projects` (case-
 * insensitive, trimmed).
 */
export function uniqueName(base: string, projects: ReadonlyArray<{ name: string }>): string {
  const name = base.trim() || "Untitled";
  const taken = new Set(projects.map((p) => p.name.trim().toLowerCase()));
  if (!taken.has(name.toLowerCase())) return name;
  for (let i = 2; ; i++) {
    const candidate = `${name} ${i}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
}

/** Default name for a copy of `name`: `"<name> copy"` (made unique). */
export function copyName(name: string, projects: ReadonlyArray<{ name: string }>): string {
  return uniqueName(`${name} copy`, projects);
}

/** Relative-ish, locale-formatted save time for the list (`now` injectable for tests). */
export function formatModified(ms: number, now: number = Date.now()): string {
  const diff = now - ms;
  if (diff >= 0 && diff < 60_000) return "just now";
  if (diff >= 0 && diff < 3_600_000) return `${Math.floor(diff / 60_000)} min ago`;
  const d = new Date(ms);
  const sameDay = new Date(now).toDateString() === d.toDateString();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}
