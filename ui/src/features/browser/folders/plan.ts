/**
 * What a folder import copies: the supported audio files (by extension, as
 * the engine decodes them; not empty, at most `MAX_IMPORT_BYTES`, shown by the library's
 * file filter), the rest skipped and counted; and whether the copy fits in the
 * browser's storage (`navigator.storage.estimate()`, checked before anything is written).
 */
import { MAX_IMPORT_BYTES, isAudioName } from "@/features/import/sources";
import { formatBytes as formatSize, t } from "@/i18n";
import type { FolderFile, PickedFolder } from "./walk";

export interface ImportPlan {
  name: string;
  /** Copied, in path order. */
  files: FolderFile[];
  /** Not supported audio (other formats, empty, too large) or hidden by the file filter. */
  skipped: number;
  /** Bytes copied. */
  bytes: number;
  /** The largest copied file (its upload is staged next to the copy). */
  largest: number;
}

/** `visible`: the library file filter's verdict per path (`Browser::CheckFileFilter`; none: all). */
export function planImport(folder: PickedFolder, visible?: (path: string) => boolean): ImportPlan {
  const files = folder.files
    .filter((f) => isAudioName(f.path) && f.file.size > 0 && f.file.size <= MAX_IMPORT_BYTES && (visible?.(f.path) ?? true))
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

/** `1.5 MB` (binary units, in the current language). */
export const formatBytes = (n: number): string => formatSize(n, { binary: true });

/** Why the copy doesn't fit (`null`: it fits, or the browser doesn't say). */
export function quotaProblem(plan: ImportPlan, estimate: StorageEstimate | null): string | null {
  if (!estimate || typeof estimate.quota !== "number") return null;
  const free = Math.max(0, estimate.quota - (estimate.usage ?? 0));
  const need = neededBytes(plan);
  if (need <= free) return null;
  return t("browser.import.no_space", { name: plan.name, need: formatBytes(need), free: formatBytes(free) });
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
  return t("browser.import.skipped", { count: skipped });
}
