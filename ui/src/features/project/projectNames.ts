// Pure helpers for the project manager (tested in projectNames.test.ts).
import type { ProjectSummary } from "@/generated";
import { formatDate, intlLocale, t } from "@/i18n";

/** Most recently saved first, then by name. */
export function sortProjects(projects: ReadonlyArray<ProjectSummary>): ProjectSummary[] {
  return [...projects].sort((a, b) => b.modified_ms - a.modified_ms || a.name.localeCompare(b.name));
}

/**
 * `base`, or `base 2`, `base 3`, ... : the first name not used by `projects` (case-
 * insensitive, trimmed).
 */
export function uniqueName(base: string, projects: ReadonlyArray<{ name: string }>): string {
  const name = base.trim() || t("project.names.untitled");
  const taken = new Set(projects.map((p) => p.name.trim().toLowerCase()));
  if (!taken.has(name.toLowerCase())) return name;
  for (let i = 2; ; i++) {
    const candidate = `${name} ${i}`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
}

/**
 * Default name of a new project: today's date in the user's locale (`toLocaleDateString`)
 * then "project" (default names are lowercase), e.g. "01/05/2026 project", numbered when
 * taken: "01/05/2026 project (2)", "(3)", ...
 */
export function dateName(projects: ReadonlyArray<{ name: string }>, now: Date = new Date(), locale?: string): string {
  const base = t("project.names.dated", { date: now.toLocaleDateString(locale ?? intlLocale()) });
  const taken = new Set(projects.map((p) => p.name.trim().toLowerCase()));
  if (!taken.has(base.toLowerCase())) return base;
  for (let i = 2; ; i++) {
    const candidate = `${base} (${i})`;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
}

/** Default name for a copy of `name`: `"<name> copy"` (made unique). */
export function copyName(name: string, projects: ReadonlyArray<{ name: string }>): string {
  return uniqueName(t("project.names.copy", { name }), projects);
}

/** Relative-ish, locale-formatted save time for the list (`now` injectable for tests). */
export function formatModified(ms: number, now: number = Date.now()): string {
  const diff = now - ms;
  if (diff >= 0 && diff < 60_000) return t("project.time.just_now");
  if (diff >= 0 && diff < 3_600_000) return t("project.time.minutes_ago", { count: Math.floor(diff / 60_000) });
  const d = new Date(ms);
  const sameDay = new Date(now).toDateString() === d.toDateString();
  return sameDay ? formatDate(d, { hour: "2-digit", minute: "2-digit" }) : formatDate(d, { year: "numeric", month: "short", day: "numeric" });
}
