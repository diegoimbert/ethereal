/**
 * "Set Value…" of an automation point: the value popover (`ValueEditPopover`) at the point.
 */

import { useState } from "react";
import type { AutomationPointId, ParamInfo } from "@/generated";
import { ValueEditPopover } from "./ValueEditPopover";

/** An open "Set Value…": the points, the clicked point's plain value, and where (lane px). */
export interface PointValueEdit {
  ids: AutomationPointId[];
  plain: number;
  x: number;
  y: number;
}

export interface PointValuePopoverProps {
  edit: PointValueEdit | null;
  info: ParamInfo;
  /** Sets the points to a plain value (clamped to the param's range). */
  onApply: (ids: AutomationPointId[], plain: number) => void;
  /** Closes it (applied or cancelled). */
  onDone: () => void;
}

export function PointValuePopover({ edit, info, onApply, onDone }: PointValuePopoverProps) {
  // Where it is, kept through the exit animation.
  const [at, setAt] = useState<PointValueEdit | null>(null);
  if (edit && at !== edit) setAt(edit);
  return (
    <div className="eth-auto-lane__value-anchor" style={at ? { left: at.x, top: at.y } : undefined}>
      <ValueEditPopover edit={edit} info={info} onApply={(plain) => edit && onApply(edit.ids, plain)} onDone={onDone} />
    </div>
  );
}
