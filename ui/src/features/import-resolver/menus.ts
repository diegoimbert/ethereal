/**
 * Right-click "Resolve…" entries for missing items, wherever they appear (owner request):
 * a missing plugin's device card / header and its "missing" badge, a missing audio clip
 * (arranger and audio editor), the missing-sample notice and Relink rows, and the import
 * report's plugin and audio file lines. Each opens the resolver focused on that item.
 */
import type { Clip, Device, MediaId, Project } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { useMediaRefs } from "@/features/media-refs/store";
import type { EngineTransport } from "@/transport";
import { catalogNow, isMissingPlugin } from "./catalog";
import { openReplace } from "./replace";
import { openResolver } from "./store";
import { t } from "@/i18n";

/** "Resolve…" (in the current language: call it when the menu is built). */
export const resolveLabel = (): string => t("resolver.menu.resolve");
/** The top-left menu and palette entry that reopens the all-missing modal (shown only when something is missing). */
export const resolveMissingLabel = (): string => t("resolver.menu.resolve_missing");
export const replacePluginLabel = (): string => t("resolver.menu.replace_plugin");

/** "Replace plugin…" for any plugin device, on hosts with a plugin catalog. */
export function replacePluginEntries(device: Device, transport: EngineTransport | null | undefined): ContextMenuEntry[] {
  if (device.kind.type !== "Plugin" || catalogNow(transport) === null) return [];
  return [{ label: replacePluginLabel(), onSelect: () => void openReplace(device.id, transport ?? undefined) }];
}

/** "Resolve…" for a plugin device whose plugin isn't installed, or a missing device (placeholder). */
export function resolvePluginEntries(device: Device, transport: EngineTransport | null | undefined): ContextMenuEntry[] {
  if (device.kind.type !== "Missing" && !isMissingPlugin(device, catalogNow(transport))) return [];
  return [{ label: resolveLabel(), onSelect: () => void openResolver({ device: device.id }) }];
}

/** "Resolve…" for a missing audio file. */
export function resolveMediaEntries(media: MediaId | null | undefined): ContextMenuEntry[] {
  if (!media || !useMediaRefs.getState().missing.has(media)) return [];
  return [{ label: resolveLabel(), onSelect: () => void openResolver({ media }) }];
}

/** "Resolve…" for an audio clip whose sample is missing. */
export function resolveClipEntries(clip: Clip): ContextMenuEntry[] {
  return clip.content.type === "Audio" ? resolveMediaEntries(clip.content.media) : [];
}

/** `entries` with the clip's "Resolve…" added before the last group (the Delete entries). */
export function withResolveClipEntries(entries: ReadonlyArray<ContextMenuEntry>, clip: Clip): ContextMenuEntry[] {
  const extra = resolveClipEntries(clip);
  if (extra.length === 0) return [...entries];
  const lastSep = entries.lastIndexOf("separator");
  if (lastSep < 0) return entries.length ? [...entries, "separator", ...extra] : extra;
  return [...entries.slice(0, lastSep), "separator", ...extra, ...entries.slice(lastSep)];
}

/** "Resolve…" for an import report line about a plugin or an audio file (by its subject). */
export function resolveReportEntries(subject: string | null): ContextMenuEntry[] {
  return [{ label: resolveLabel(), onSelect: () => void openResolver(subject ? { name: subject } : undefined) }];
}

/** Command palette entry (same shape as the shell's `PaletteCommand`), when something may be missing. */
export function resolverCommands(
  transport: EngineTransport,
  project: Project,
): Array<{ id: string; label: string; group: string; keywords?: string; run(): void }> {
  const catalog = catalogNow(transport);
  const plugins = Object.values(project.devices).some((d) => d.kind.type === "Missing" || isMissingPlugin(d, catalog));
  if (!plugins && useMediaRefs.getState().missing.size === 0) return [];
  return [
    {
      id: "resolve:missing",
      group: "Project",
      label: resolveMissingLabel(),
      keywords: "missing plugin device sample audio file replace relink resolve import find later",
      run: () => void openResolver(undefined, transport),
    },
  ];
}
