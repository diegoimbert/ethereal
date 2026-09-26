// Pure helpers for browsing engine-side locations (relative paths only; tested in paths.test.ts).
import type { BrowseLocation, DirectoryEntry, MediaRef, MediaSource } from "@/generated";

/** `"Drums/Loops"` → `["Drums", "Drums/Loops"]` (cumulative relative paths of each segment). */
export function pathSegments(path: string): { name: string; path: string }[] {
  const parts = path.split("/").filter(Boolean);
  return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join("/") }));
}

/** Parent folder of a relative path (`""` for top level). */
export function parentPath(path: string): string {
  const parts = path.split("/").filter(Boolean);
  parts.pop();
  return parts.join("/");
}

export function sameLocation(a: BrowseLocation, b: BrowseLocation): boolean {
  if (a.type === "Library" && b.type === "Library") return a.id === b.id;
  return a.type === b.type;
}

/** Stable string key of a location. */
export function locationKey(l: BrowseLocation): string {
  return l.type === "Library" ? `library:${l.id}` : "project-media";
}

/**
 * The engine source of a listed file. Files in the project's own media folder refer to the
 * existing `MediaRef` when it can be found (`MediaRef.file` = `media/<path>`), so dropping
 * them doesn't import a second copy.
 */
export function sourceOf(
  location: BrowseLocation,
  entry: Pick<DirectoryEntry, "path">,
  media: Readonly<Record<string, MediaRef>> | undefined,
): MediaSource {
  if (location.type === "ProjectMedia" && media) {
    const file = `media/${entry.path}`;
    const found = Object.values(media).find((m) => m.file === file);
    if (found) return { type: "Project", media: found.id };
  }
  return { type: "Location", location, path: entry.path };
}

/** `"1.2 MB"` */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}
