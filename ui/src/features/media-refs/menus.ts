/**
 * Media-reference entries of the app's menus, following the owner's patterns (like the
 * freeze entries): the clip menu gets "Relink Sample…" when its sample is missing, inserted
 * before the menu's last group (the destructive "Delete" entries); the command palette gets
 * "Relink missing samples…" and "Collect All and Save".
 */

import type { Clip, Project } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import type { EngineTransport } from "@/transport";
import { collectAll, openRelink, useMediaRefs } from "./store";

function insertBeforeLastGroup(entries: ReadonlyArray<ContextMenuEntry>, extra: ContextMenuEntry[]): ContextMenuEntry[] {
  if (extra.length === 0) return [...entries];
  const lastSep = entries.lastIndexOf("separator");
  if (lastSep < 0) return entries.length ? [...entries, "separator", ...extra] : extra;
  return [...entries.slice(0, lastSep), "separator", ...extra, ...entries.slice(lastSep)];
}

/** Clip-menu entries for `clip` (an audio clip whose sample is missing). */
export function mediaRefClipEntries(clip: Clip): ContextMenuEntry[] {
  if (clip.content.type !== "Audio") return [];
  const media = clip.content.media;
  if (!useMediaRefs.getState().missing.has(media)) return [];
  return [{ label: "Relink Sample…", onSelect: () => openRelink(media) }];
}

export function withMediaRefClipEntries(entries: ReadonlyArray<ContextMenuEntry>, clip: Clip): ContextMenuEntry[] {
  return insertBeforeLastGroup(entries, mediaRefClipEntries(clip));
}

/** A command palette entry (same shape as the shell's `PaletteCommand`). */
export interface MediaRefPaletteCommand {
  id: string;
  label: string;
  group: string;
  keywords?: string;
  run(): void;
}

/** Palette entries for the open project. */
export function mediaRefCommands(transport: EngineTransport, project: Project): MediaRefPaletteCommand[] {
  const out: MediaRefPaletteCommand[] = [];
  const missing = useMediaRefs.getState().missing.size;
  if (missing > 0) {
    out.push({
      id: "media:relink",
      group: "Project",
      label: "Relink missing samples…",
      keywords: "missing media file sample relink locate find offline",
      run: () => openRelink(),
    });
  }
  if (Object.values(project.media).some((m) => m.location.type === "External")) {
    out.push({
      id: "media:collect",
      group: "Project",
      label: "Collect All and Save",
      keywords: "collect consolidate copy samples media files project folder save",
      run: () => void collectAll(transport),
    });
  }
  return out;
}
