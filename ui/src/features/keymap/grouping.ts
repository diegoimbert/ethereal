import { useMemo, useSyncExternalStore } from "react";
import { registryVersion, subscribeRegistry, type KeymapAction } from "./registry";
import { allActions, useKeymapStore } from "./store";

/** Group order in the editor and the cheat sheet; other groups follow, by name. */
export const GROUP_ORDER = ["Transport", "Edit", "Arrangement", "Time selection", "Piano roll", "Automation", "View", "App"];

/** Actions grouped for display (groups in `GROUP_ORDER`, then by name; rows in registry order). */
export function groupActions(actions: ReadonlyArray<KeymapAction>): Array<{ group: string; actions: KeymapAction[] }> {
  const groups = new Map<string, KeymapAction[]>();
  for (const a of actions) {
    const list = groups.get(a.group) ?? [];
    list.push(a);
    groups.set(a.group, list);
  }
  const rank = (g: string) => {
    const i = GROUP_ORDER.indexOf(g);
    return i < 0 ? GROUP_ORDER.length : i;
  };
  return [...groups.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
    .map(([group, list]) => ({ group, actions: list }));
}

/** Every action, with the palette's own ones refreshed (re-renders on registry/keymap changes). */
export function useActions(): KeymapAction[] {
  const version = useSyncExternalStore(subscribeRegistry, registryVersion);
  const paletteVersion = useKeymapStore((s) => s.paletteVersion);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- recomputed when either version moves
  return useMemo(() => allActions(), [version, paletteVersion]);
}

