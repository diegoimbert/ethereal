import clsx from "clsx";
import type { MouseEvent, ReactNode } from "react";
import type { AutomationTarget, Device, ParamInfo } from "@/generated";
import { openContextMenu } from "@/kit";
import { useProjectStore } from "@/state";
import { midiTarget } from "@/features/midi-learn/targets";
import { targetKey } from "@/features/automation/params";
import { useAutomationUi } from "@/features/automation/uiStore";
import type { ParamBinding } from "./context";
import { modTargetId, ParamModulation } from "./seams";

export function paramTarget(device: Device, info: ParamInfo): AutomationTarget {
  return { type: "DeviceParam", device: device.id, param: info.id };
}

/** Is an enabled automation lane driving this param? */
export function useAutomated(device: Device, info: ParamInfo): boolean {
  return useProjectStore((s) => {
    const lanes = s.project?.automation_lanes;
    if (!lanes) return false;
    for (const id in lanes) {
      const t = lanes[id]!.target;
      if (lanes[id]!.enabled && t.type === "DeviceParam" && t.device === device.id && t.param === info.id) return true;
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

export interface ParamShellProps {
  binding: ParamBinding;
  /** Widget kind modifier (`eth-param--knob`, ...). */
  kind: string;
  className?: string;
  children: ReactNode;
  /** Element: a `label` wraps form controls (choice). */
  as?: "div" | "label";
}

/**
 * The frame of every param widget: MIDI-learn target (`midiTarget()`), modulation drop
 * target + depth-ring slot, automation indicator and the param context menu.
 */
export function ParamShell({ binding: b, kind, className, children, as: Tag = "div" }: ParamShellProps) {
  const automated = useAutomated(b.device, b.info);
  const target = midiTarget({ type: "Param", target: { type: "DeviceParam", device: b.device.id, param: b.info.id } });
  return (
    <Tag
      className={clsx("eth-param", `eth-param--${kind}`, automated && "eth-param--automated", className)}
      data-param={b.info.id}
      data-mod-target={modTargetId(b.device.id, b.info.id)}
      onContextMenu={(e) => paramMenu(e, b)}
      {...target}
    >
      {children}
      {automated && <span className="eth-param__auto" role="img" aria-label={`${b.info.name} is automated`} title="Automated" />}
      {ParamModulation ? (
        <ParamModulation device={b.device} info={b.info} normalized={b.normalized} />
      ) : (
        <span className="eth-param__mod" aria-hidden="true" />
      )}
    </Tag>
  );
}
