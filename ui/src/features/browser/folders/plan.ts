/**
 * What a folder import copies (`base-136`): the supported audio files (by extension, as
 * the engine decodes them; not empty, at most `MAX_IMPORT_BYTES`), the rest skipped and
 * counted; and whether the copy fits in the browser's storage (`navigator.storage.estimate()`,
 * checked before anything is written).
 */
import { MAX_IMPORT_BYTES, isAudioName } from "@/features/import/sources";
import type { FolderFile, PickedFolder } from "./walk";

export interface ImportPlan {
  name: string;
  /** Copied, in path order. */
  files: FolderFile[];
  /** Not supported audio (other formats, empty, too large). */
  skipped: number;
  /** Bytes copied. */
  bytes: number;
  /** The largest copied file (its upload is staged next to the copy). */
  largest: number;
}

export function planImport(folder: PickedFolder): ImportPlan {
  const files = folder.files
    .filter((f) => isAudioName(f.path) && f.file.size > 0 && f.file.size <= MAX_IMPORT_BYTES)
    .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  return {
    name: folder.name,
    files,
    skipped: folder.files.length - files.length,
    bytes: files.reduce((n, f) => n + f.file.size, 0),
    largest: files.reduce((n, f) => Math.max(n, f.file.size), 0),
  };
}

/** `navigator.storage.estimate()`'s answer (`null`: unknown). */
export interface StorageEstimate {
  quota?: number;
  usage?: number;
}

/** Headroom kept free besides the copy (projects, caches and the index grow too). */
export const QUOTA_MARGIN_BYTES = 64 * 1024 * 1024;

/** Space the import needs: the copy, plus one file's upload staging, plus the margin. */
export const neededBytes = (plan: ImportPlan) => plan.bytes + plan.largest + QUOTA_MARGIN_BYTES;

export function formatBytes(n: number): string {
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(1)} GB`;
  if (n >= 1024 ** 2) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1024))} KB`;
}

/** Why the copy doesn't fit (`null`: it fits, or the browser doesn't say). */
export function quotaProblem(plan: ImportPlan, estimate: StorageEstimate | null): string | null {
  if (!estimate || typeof estimate.quota !== "number") return null;
  const free = Math.max(0, estimate.quota - (estimate.usage ?? 0));
  const need = neededBytes(plan);
  if (need <= free) return null;
  return `“${plan.name}” needs ${formatBytes(need)} of browser storage but only ${formatBytes(free)} is free. Import a smaller folder, or remove imported folders you no longer use.`;
}

/** The browser's storage estimate, or `null` where it can't tell. */
export async function estimateStorage(nav: Navigator | undefined = globalThis.navigator): Promise<StorageEstimate | null> {
  try {
    return (await nav?.storage?.estimate?.()) ?? null;
  } catch {
    return null;
  }
}

/** "3 files skipped (not supported audio)". */
export function skippedText(skipped: number): string {
  return skipped === 1 ? "1 file skipped (not supported audio)" : `${skipped} files skipped (not supported audio)`;
}
