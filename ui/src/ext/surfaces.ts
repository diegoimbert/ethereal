// Helpers for the edit surfaces' extension points (`@/ext`): menu entries and activity.
import type { ContextMenuEntry } from "@/kit";
import { contributions, emit } from "./index";
import type { EditActivity, EditSurface } from "./points";

/** The extensions' entries of a surface's context menu (`surface.menu`), after a separator. */
export function surfaceMenuEntries(surface: EditSurface, at: { clientX: number; clientY: number }): ContextMenuEntry[] {
  const entries = contributions("surface.menu").flatMap((f) => [...f(surface, at)]);
  return entries.length ? ["separator", ...entries] : [];
}

/** The extensions' entries of the arranger ruler's menu at `beats` (`arrangement.rulerMenu`). */
export function rulerMenuEntries(beats: number): ContextMenuEntry[] {
  return contributions("arrangement.rulerMenu").flatMap((f) => [...f(beats)]);
}

/** What the user is doing on the arrangement now (`edit.activity`; `null`: done). */
export function setEditActivity(activity: EditActivity | null): void {
  emit("edit.activity", activity);
}
