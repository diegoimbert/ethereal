/**
 * "Assign to macro ›" in a param's context menu (`paramMenuEntries`, picked up by the device
 * renderer's seam): the macros of every rack holding the device (and, for a rack's own params
 * after its macros, that rack's), one click maps the param to a macro. Only a rack's shown
 * macros (`RackMacros::visible`) are listed, under their names.
 */
import type { Command, Device, ParamInfo, Project } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { useMacroRename } from "@/features/devices/layout/macros";
import { cmd, newId } from "@/transport";
import { t } from "@/i18n";
import {
  canTarget,
  DEFAULT_DEPTH,
  DEVICE_ENABLED_PARAM,
  hasMacros,
  macroIndex,
  rackOf,
  sameSource,
  sourceName,
  sourceNameIsUserText,
  switchDepth,
  visibleMacros,
} from "./model";

/** The racks (drum racks included) whose macros can reach `device`'s `param`, nearest first. */
function reachingRacks(project: Project, device: Device, param: number): Device[] {
  const out: Device[] = [];
  // A rack's own params after its macros, never its switch.
  if (hasMacros(device) && macroIndex(device, param) === null && param !== DEVICE_ENABLED_PARAM) out.push(device);
  let cur: Device | undefined = device;
  for (let i = 0; cur && i < 16; i++) {
    const host = rackOf(project, cur);
    if (host === null) break;
    cur = project.devices[host];
    if (cur && hasMacros(cur)) out.push(cur);
  }
  return out;
}

/**
 * The context menu entries for `device`'s `info`: "rename macro" on a rack's own macro (the
 * `Macros` widget edits the name in place), then "assign to macro ›" (none outside a rack).
 */
export function paramMenuEntries(device: Device, info: ParamInfo, send: (c: Command) => unknown): ContextMenuEntry[] {
  const index = macroIndex(device, info.id);
  const rename: ContextMenuEntry[] =
    index === null ? [] : ["separator", { label: t("modulation.macro.rename"), onSelect: () => useMacroRename.getState().start(device.id, index) }];
  return [...rename, ...assignEntries(device, info.id, DEFAULT_DEPTH, send)];
}

/**
 * "Assign to macro ›" on a device's on/off button: the macro switches the device
 * (`DEVICE_ENABLED_PARAM`, depth [`switchDepth`]).
 */
export function switchMenuEntries(device: Device, send: (c: Command) => unknown): ContextMenuEntry[] {
  return assignEntries(device, DEVICE_ENABLED_PARAM, switchDepth(device), send);
}

function assignEntries(device: Device, param: number, depth: number, send: (c: Command) => unknown): ContextMenuEntry[] {
  const project = useProjectStore.getState().project;
  if (!project) return [];
  const info = { id: param };
  const racks = reachingRacks(project, device, info.id);
  if (racks.length === 0) return [];
  const macros = (rack: Device): ContextMenuEntry[] =>
    Array.from({ length: visibleMacros(rack) }, (_, index) => {
      const source = { type: "Macro", rack: rack.id, index } as const;
      const mapped = Object.values(project.mod_mappings).some((m) => sameSource(m.source, source) && m.device === device.id && m.param === info.id);
      return {
        label: sourceName(project, source),
        userText: sourceNameIsUserText(project, source) || undefined,
        disabled: mapped || !canTarget(project, source, device, info.id),
        reason: mapped ? t("modulation.macro.assigned") : undefined,
        onSelect: () => void send(cmd("Modulation", { type: "Map", id: newId(), source, device: device.id, param: info.id, depth })),
      };
    });
  const submenu: ContextMenuEntry[] =
    racks.length === 1 ? macros(racks[0]!) : racks.map((rack) => ({ label: rack.name, userText: true, onSelect: () => {}, submenu: macros(rack) }));
  return ["separator", { label: t("modulation.macro.assign"), onSelect: () => {}, submenu }];
}
