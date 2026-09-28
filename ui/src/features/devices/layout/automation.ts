/**
 * Automation affordances of param widgets: the "automated" state, opening a param's lane in
 * the arrangement, and the param context menu.
 */

import type { MouseEvent } from "react";
import type { AutomationTarget, Device, ParamInfo } from "@/generated";
import { openContextMenu } from "@/kit";
import { useProjectStore } from "@/state";
import { targetKey } from "@/features/automation/params";
import { useAutomationUi } from "@/features/automation/uiStore";
import type { ParamBinding } from "./context";

export function paramTarget(device: Device, info: ParamInfo): AutomationTarget {
  return { type: "DeviceParam", device: device.id, param: info.id };
}

/** Is an enabled automation lane driving this param? */
export function useAutomated(device: Device, info: ParamInfo): boolean {
  return useProjectStore((s) => {
    const lanes = s.project?.automation_lanes;
    if (!lanes) return false;
    for (const id in lanes) {
      const l = lanes[id]!;
      const t = l.target;
      if (l.enabled && t.type === "DeviceParam" && t.device === device.id && t.param === info.id) return true;
    }
    return false;
  });
}

/** Open the param's automation lane in the arrangement (shows the track's automation). */
export function showAutomation(device: Device, info: ParamInfo): void {
  useAutomationUi.getState().show(device.track, targetKey(paramTarget(device, info)));
}

/** The param context menu: automation and reset (MIDI learn has its own mode menu). */
export function paramMenu(e: MouseEvent, b: ParamBinding): void {
  openContextMenu(e, [
    { label: "Show automation lane", disabled: !b.info.automatable, onSelect: () => showAutomation(b.device, b.info) },
    "separator",
    { label: "Reset to default", disabled: b.plain === b.info.default, onSelect: b.reset },
  ]);
}
