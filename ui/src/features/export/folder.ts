/**
 * Desktop: the folder exports go to (`ExportRequest.folder`), chosen in the OS folder dialog
 * and remembered on this machine; none = the project's `exports/` folder. The exported files
 * can be shown in the OS file manager (`Media::Reveal`).
 */
import type { RevealTarget } from "@/generated";
import type { EngineTransport } from "@/transport";

/** localStorage: the chosen export folder (an absolute path on this machine). */
export const FOLDER_KEY = "eth.export.folder";

export function loadExportFolder(): string | null {
  try {
    return localStorage.getItem(FOLDER_KEY) || null;
  } catch {
    return null;
  }
}

export function saveExportFolder(folder: string | null): void {
  try {
    if (folder) localStorage.setItem(FOLDER_KEY, folder);
    else localStorage.removeItem(FOLDER_KEY);
  } catch {
    // Not remembered: this export still goes there.
  }
}

/** The desktop transport's folder dialog (`TauriTransport.pickFolder`), duck-typed. */
export function folderPicker(t: EngineTransport): { pickFolder(title?: string): Promise<string | null> } | null {
  const p = t as unknown as { pickFolder?: unknown };
  return typeof p.pickFolder === "function" ? (t as unknown as { pickFolder(title?: string): Promise<string | null> }) : null;
}

/** An absolute path (`/x`, `C:\x`, `\\server\x`) rather than a project-relative one. */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith("/") || path.startsWith("\\\\") || /^[A-Za-z]:[\\/]/.test(path);
}

/** What shows an exported file (`ExportResult.Files` entry) in the file manager. */
export function exportedFileTarget(file: string): RevealTarget {
  return isAbsolutePath(file) ? { type: "Media", source: { type: "Path", path: file } } : { type: "ProjectFile", path: file };
}
