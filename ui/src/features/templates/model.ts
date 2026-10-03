/**
 * Templates (v0.3, `templates`; CONTRACTS.md §13.10): the dialog state, placement and the
 * entry points other features call (track context menu, palette, project screen).
 *
 * The engine owns the files (factory track templates embedded, user templates in the user
 * library); the UI lists, saves, inserts and creates projects through `Template::*`.
 */

import { create } from "zustand";
import type { MouseEvent } from "react";
import type { Command, Project, TemplateInfo, TemplateKind, Track, TrackId } from "@/generated";
import { deriveId } from "@/features/comping/model";
import { openContextMenu, type ContextMenuEntry } from "@/kit";
import { cmd, newId, nextGestureId, type EngineTransport } from "@/transport";

/** Where inserted tracks go (`Template::Insert` placement). */
export interface Placement {
  parent: TrackId | null;
  before: TrackId | null;
}

export type TemplateDialog =
  | { type: "save-tracks"; tracks: TrackId[]; name: string }
  | { type: "save-project"; name: string }
  | { type: "insert"; placement: Placement }
  | { type: "rename"; template: TemplateInfo }
  | { type: "delete"; template: TemplateInfo };

interface TemplateDialogState {
  dialog: TemplateDialog | null;
  open(dialog: TemplateDialog): void;
  close(): void;
}

export const useTemplateDialog = create<TemplateDialogState>((set) => ({
  dialog: null,
  open: (dialog) => set({ dialog }),
  close: () => set({ dialog: null }),
}));

export const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** A user template of `kind` with this name (case-insensitive), except `except`. */
export function findByName(templates: ReadonlyArray<TemplateInfo>, kind: TemplateKind, name: string, except?: string): TemplateInfo | undefined {
  const n = name.trim().toLowerCase();
  if (!n) return undefined;
  return templates.find((t) => !t.factory && t.kind === kind && t.name.toLowerCase() === n && t.id !== except);
}

/** Factory templates first, then the user's (the engine already sorts each by name). */
export function groupTemplates(templates: ReadonlyArray<TemplateInfo>): { factory: TemplateInfo[]; user: TemplateInfo[] } {
  return { factory: templates.filter((t) => t.factory), user: templates.filter((t) => !t.factory) };
}

/** Case-insensitive match on name, tags and description. */
export function matchesQuery(t: TemplateInfo, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return t.name.toLowerCase().includes(q) || t.meta.tags.some((tag) => tag.includes(q)) || (t.meta.description ?? "").toLowerCase().includes(q);
}

function siblingsOf(project: Project, parent: TrackId | null): Track[] {
  return Object.values(project.tracks)
    .filter((t) => t.parent === parent)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
}

/** Right after `track`, among its siblings. */
export function placementAfter(project: Project, track: Track): Placement {
  const sibs = siblingsOf(project, track.parent);
  const next = sibs[sibs.findIndex((t) => t.id === track.id) + 1];
  return { parent: track.parent, before: next?.id ?? null };
}

/** The tracks a "Save as template" from `track`'s menu saves: the selection if it holds `track`. */
export function tracksToSave(track: TrackId, selected: ReadonlySet<TrackId>, project: Project): TrackId[] {
  const pick = selected.has(track) ? [...selected] : [track];
  return pick.filter((id) => project.tracks[id] && project.tracks[id].kind !== "Master");
}

/**
 * Insert track template `template` (one undo step). Resolves with the first new track's id
 * (`derive_id(seed, 0)`).
 */
export async function insertTemplate(transport: EngineTransport, template: string, placement: Placement): Promise<TrackId> {
  const seed = newId();
  const gesture = nextGestureId();
  try {
    await transport.send(cmd("Template", { type: "Insert", template, seed, parent: placement.parent, before: placement.before }), { gesture });
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
  return deriveId(seed, 0);
}

/** "Save as template…" and "Insert track template…" for a track's context menu. */
export function templateTrackEntries(project: Project | null, track: Track, selected: ReadonlySet<TrackId>): ContextMenuEntry[] {
  if (!project || track.kind === "Master") return [];
  const tracks = tracksToSave(track.id, selected, project);
  const open = useTemplateDialog.getState().open;
  return [
    "separator",
    {
      label: tracks.length > 1 ? `Save ${tracks.length} Tracks as Template…` : "Save as Template…",
      onSelect: () => open({ type: "save-tracks", tracks, name: tracks.length === 1 ? (project.tracks[tracks[0]!]?.name ?? "") : "" }),
    },
    { label: "Insert Track Template…", onSelect: () => open({ type: "insert", placement: placementAfter(project, track) }) },
  ];
}

/** `null` = an empty project; otherwise a project template id. */
export type ProjectTemplateChoice = string | null;

/**
 * The command creating project `id` for a picker value: the default template (or an empty
 * project when there is none) while untouched, an empty project, or the picked template.
 */
export function newProjectCommand(id: string, name: string, value: ProjectTemplateChoice | undefined): Command {
  if (value === undefined) return cmd("Template", { type: "NewProject", id, name, template: null });
  if (value === null) return cmd("Project", { type: "Create", id, name });
  return cmd("Template", { type: "NewProject", id, name, template: value });
}

/** Context-menu entries of a user template (Rename…, Delete…; project templates: default). */
export function templateMenu(e: Pick<MouseEvent, "clientX" | "clientY" | "preventDefault" | "stopPropagation">, t: TemplateInfo, setDefault?: (id: string | null) => void): void {
  const open = useTemplateDialog.getState().open;
  openContextMenu(e, [
    ...(setDefault && t.kind === "Project"
      ? [
          t.default
            ? { label: "Stop Using as Default", onSelect: () => setDefault(null) }
            : { label: "Use for New Projects", onSelect: () => setDefault(t.id) },
          "separator" as const,
        ]
      : []),
    ...(t.factory
      ? []
      : [
          { label: "Rename…", onSelect: () => open({ type: "rename", template: t }) },
          "separator" as const,
          { label: "Delete Template…", danger: true, onSelect: () => open({ type: "delete", template: t }) },
        ]),
  ]);
}

