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
