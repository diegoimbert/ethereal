import { useMemo, useSyncExternalStore } from "react";
import { registryVersion, subscribeRegistry, type KeymapAction } from "./registry";
import { allActions, useKeymapStore } from "./store";

export { actionLabel, GROUP_ORDER, groupActions, groupLabel, scopesLabel } from "./groups";

/** Every action, with the palette's own ones refreshed (re-renders on registry/keymap changes). */
export function useActions(): KeymapAction[] {
  const version = useSyncExternalStore(subscribeRegistry, registryVersion);
  const paletteVersion = useKeymapStore((s) => s.paletteVersion);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- recomputed when either version moves
  return useMemo(() => allActions(), [version, paletteVersion]);
}
