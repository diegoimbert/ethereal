/**
 * "Assign to macro ›" in a param's context menu (`paramMenuEntries`, picked up by the device
 * renderer's seam): the macros of every rack holding the device (and, for a rack's own params
 * after its macros, that rack's), one click maps the param to a macro.
 */
import type { Command, Device, ParamInfo, Project } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, newId } from "@/transport";
import { t } from "@/i18n";
import { canTarget, DEFAULT_DEPTH, isChainRack, MACROS, rackOf, sameSource, SELECTOR_PARAM, sourceName } from "./model";

/** The racks whose macros can reach `device`'s `param`, nearest first. */
function reachingRacks(project: Project, device: Device, param: number): Device[] {
  const out: Device[] = [];
  if (isChainRack(device) && param >= SELECTOR_PARAM) out.push(device);
  let cur: Device | undefined = device;
  for (let i = 0; cur && i < 16; i++) {
    const host = rackOf(project, cur);
    if (host === null) break;
    cur = project.devices[host];
    if (cur && isChainRack(cur)) out.push(cur);
  }
  return out;
}

/** The context menu entries for `device`'s `info` (none outside a rack). */
export function paramMenuEntries(device: Device, info: ParamInfo, send: (c: Command) => unknown): ContextMenuEntry[] {
  const project = useProjectStore.getState().project;
  if (!project) return [];
  const racks = reachingRacks(project, device, info.id);
  if (racks.length === 0) return [];
  const macros = (rack: Device): ContextMenuEntry[] =>
    Array.from({ length: MACROS }, (_, index) => {
      const source = { type: "Macro", rack: rack.id, index } as const;
      const mapped = Object.values(project.mod_mappings).some((m) => sameSource(m.source, source) && m.device === device.id && m.param === info.id);
      return {
        label: sourceName(project, source),
        disabled: mapped || !canTarget(project, source, device, info.id),
        reason: mapped ? t("modulation.macro.assigned") : undefined,
        onSelect: () => void send(cmd("Modulation", { type: "Map", id: newId(), source, device: device.id, param: info.id, depth: DEFAULT_DEPTH })),
      };
    });
  const submenu: ContextMenuEntry[] =
    racks.length === 1 ? macros(racks[0]!) : racks.map((rack) => ({ label: rack.name, userText: true, onSelect: () => {}, submenu: macros(rack) }));
  return ["separator", { label: t("modulation.macro.assign"), onSelect: () => {}, submenu }];
}
