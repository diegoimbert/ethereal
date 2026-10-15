/**
 * Automation affordances of param widgets: the "automated" state, opening a param's lane in
 * the arrangement, and the param context menu.
 */

import type { MouseEvent } from "react";
import type { AutomationTarget, Device, ParamInfo } from "@/generated";
import { openContextMenu } from "@/kit";
import { useProjectStore } from "@/state";
import { targetKey } from "@/features/automation/params";
import { hasEnabledLane } from "@/features/automation/live";
import { useAutomationUi } from "@/features/automation/uiStore";
import type { ParamBinding } from "./context";
import { t } from "@/i18n";

export function paramTarget(device: Device, info: ParamInfo): AutomationTarget {
  return { type: "DeviceParam", device: device.id, param: info.id };
}

/**
 * Is an enabled automation lane driving this param? One set lookup per store change (shared,
 * rebuilt only when the lanes change), not a scan of every lane per visible widget.
 */
export function useAutomated(device: Device, info: ParamInfo): boolean {
  return useProjectStore((s) => (s.project ? hasEnabledLane(s.project, paramTarget(device, info)) : false));
}

/** Open the param's automation lane in the arrangement (shows the track's automation). */
export function showAutomation(device: Device, info: ParamInfo): void {
  useAutomationUi.getState().show(device.track, targetKey(paramTarget(device, info)));
}

/**
 * The param context menu: automation and reset (MIDI learn has its own mode menu). Shared by
 * the device's widgets and the "Show all parameters" list.
 */
export function paramMenu(e: MouseEvent, b: Pick<ParamBinding, "device" | "info" | "plain" | "reset">): void {
  openContextMenu(e, [
    { label: t("devices.param.show_automation"), disabled: !b.info.automatable, onSelect: () => showAutomation(b.device, b.info) },
    "separator",
    { label: t("devices.param.reset"), disabled: b.plain === b.info.default, onSelect: b.reset },
  ]);
}
