import clsx from "clsx";
import type { ReactNode } from "react";
import { midiTarget } from "@/features/midi-learn/targets";
import { paramMenu, useAutomated } from "./automation";
import type { ParamBinding } from "./context";
import { modTargetId, ParamModulation } from "./seams";

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
