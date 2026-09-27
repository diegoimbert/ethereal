import type { DeviceDescriptor, ParamInfo, Track } from "@/generated";

/** MIME type used to drag a device within/between chains. */
export const DEVICE_DRAG_TYPE = "application/x-ethereal-device";

/** Built-in types that can go on `track` (instruments only on MIDI tracks). */
export function insertableTypes(types: ReadonlyArray<DeviceDescriptor>, track: Track): DeviceDescriptor[] {
  return types.filter(
    (d) => d.device_type.type === "Builtin" && (d.category !== "Instrument" || track.kind === "Midi"),
  );
}

/** Visible params grouped by `ParamInfo.group`, in first-appearance order. */
export function groupParams(params: ReadonlyArray<ParamInfo>): Array<{ group: string | null; params: ParamInfo[] }> {
  const out: Array<{ group: string | null; params: ParamInfo[] }> = [];
  for (const p of params) {
    if (p.hidden) continue;
    let g = out.find((x) => x.group === p.group);
    if (!g) out.push((g = { group: p.group, params: [] }));
    g.params.push(p);
  }
  return out;
}

export type ParamGroup = ReturnType<typeof groupParams>[number];

/** A device shows all its params up to this many; above it, the rest folds under "More". */
export const FOLD_OVER = 6;
/** Params in the always-visible main section (whole leading groups, at least one group). */
export const MAIN_MAX = 4;

/**
 * Split grouped params into the main section (shown large, always visible) and the rest
 * (behind "More"): small devices show everything; bigger ones keep leading whole groups
 * up to `MAIN_MAX` params.
 */
export function splitMainParams(groups: ReadonlyArray<ParamGroup>): { main: ParamGroup[]; more: ParamGroup[] } {
  const total = groups.reduce((n, g) => n + g.params.length, 0);
  if (total <= FOLD_OVER) return { main: [...groups], more: [] };
  let count = 0;
  let i = 0;
  while (i < groups.length && (i === 0 || count + groups[i]!.params.length <= MAIN_MAX)) {
    count += groups[i]!.params.length;
    i++;
  }
  return { main: groups.slice(0, i), more: groups.slice(i) };
}
