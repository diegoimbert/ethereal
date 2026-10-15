/**
 * "Set Value…" of an automation point: a small popover at the point (kit `Popover`) with
 * one field in the param's display unit. Enter applies, Escape cancels, a click outside
 * applies (like the app's other inline editors); a value that doesn't parse is dropped.
 */

import { useState, type KeyboardEvent } from "react";
import type { AutomationPointId, ParamInfo } from "@/generated";
import { Popover, TextInput } from "@/kit";
import { editText, editUnit, parseEditText } from "./params";
import { useT } from "@/i18n";

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
  const t = useT();
  // What it shows, kept through the exit animation.
  const [shown, setShown] = useState<{ edit: PointValueEdit; text: string; invalid: boolean } | null>(null);
  if (edit && shown?.edit !== edit) setShown({ edit, text: editText(info, edit.plain), invalid: false });
  const cur = edit && shown?.edit === edit ? shown : null;

  const finish = (apply: boolean, keepIfInvalid = false) => {
    if (!cur) return;
    if (apply) {
      const v = parseEditText(info, cur.text);
      if (v === null && keepIfInvalid) {
        setShown({ ...cur, invalid: true });
        return;
      }
      if (v !== null) onApply(cur.edit.ids, v);
    }
    onDone();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    // The lane's shortcuts (Delete, arrows, ⌘A...) and the popover's own Escape stay out.
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      finish(true, true);
    } else if (e.key === "Escape") {
      e.preventDefault();
      finish(false);
    }
  };

  const unit = editUnit(info);
  const at = shown?.edit;
  return (
    <div className="eth-auto-lane__value-anchor" style={at ? { left: at.x, top: at.y } : undefined}>
      <Popover
        open={edit !== null}
        // Only a click outside closes it from the popover (Escape is handled above): apply.
        onOpenChange={(o) => !o && finish(true)}
        trigger={() => null}
        aria-label={t("automation.value.set", { name: info.name })}
        className="eth-auto-value-pop"
      >
        {shown && (
          <div className="eth-auto-value-pop__row" onKeyDown={onKeyDown}>
            <TextInput
              size="sm"
              className="eth-auto-value-pop__input"
              aria-label={t("automation.value.field", { name: info.name })}
              invalid={shown.invalid}
              value={shown.text}
              onChange={(e) => setShown({ ...shown, text: e.target.value, invalid: false })}
              onFocus={(e) => e.currentTarget.select()}
              spellCheck={false}
              autoComplete="off"
              autoFocus
            />
            {unit && <span className="eth-auto-value-pop__unit">{unit}</span>}
          </div>
        )}
      </Popover>
    </div>
  );
}
