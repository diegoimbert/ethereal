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
import { valueMenuEntries } from "@/features/automation/valueMenu";
import { modulationMenuEntries } from "./seams";
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

/** The device's on/off switch as an automation target (Live's "Device On"). */
export function switchTarget(device: Pick<Device, "id">): AutomationTarget {
  return { type: "DeviceEnabled", device: device.id };
}

/** Open the device's "Device On" lane in the arrangement (a stepped off/on lane). */
export function showSwitchAutomation(device: Pick<Device, "id" | "track">): void {
  useAutomationUi.getState().show(device.track, targetKey(switchTarget(device)));
}

interface TargetMenu {
  automatable: boolean;
  showAutomation(): void;
  /** Already at its default (nothing to reset). */
  isDefault: boolean;
  reset(): void;
}

function targetMenu(e: MouseEvent, m: TargetMenu): void {
  openContextMenu(e, [
    // Not automatable: no automation entry at all.
    ...(m.automatable ? [{ label: t("devices.param.show_automation"), onSelect: m.showAutomation }, "separator" as const] : []),
    { label: t("devices.param.reset"), disabled: m.isDefault, onSelect: m.reset },
  ]);
}

/**
 * The param context menu: the app's value menu (its automation lane when automatable, set
 * value…, reset; MIDI learn has its own mode menu). Shared by the device's widgets and the
 * "Show all parameters" list.
 */
export function paramMenu(e: MouseEvent, b: Pick<ParamBinding, "device" | "info" | "plain" | "reset" | "setPlain" | "send">): void {
  e.preventDefault();
  e.stopPropagation();
  const value = valueMenuEntries(
    {
      info: b.info,
      plain: b.plain,
      set: b.setPlain,
      reset: b.reset,
      automation: b.info.automatable ? { show: () => showAutomation(b.device, b.info) } : undefined,
    },
    e.currentTarget as Element,
  );
  // Inside a rack: "assign to macro ›" (modulation's seam).
  openContextMenu(e, [...value, ...modulationMenuEntries(b.device, b.info, b.send)]);
}

/**
 * The same menu on a device's on/off button: its "Device On" lane, and reset (= on, the
 * switch's default).
 */
export function switchMenu(e: MouseEvent, device: Pick<Device, "id" | "track" | "enabled">, setEnabled: (on: boolean) => void): void {
  targetMenu(e, {
    automatable: true,
    showAutomation: () => showSwitchAutomation(device),
    isDefault: device.enabled,
    reset: () => setEnabled(true),
  });
}
