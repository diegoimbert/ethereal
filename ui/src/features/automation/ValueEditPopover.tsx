/**
 * "Set value…": a small popover (kit `Popover`) with one field in the value's display unit
 * (`ParamInfo`). Enter applies, Escape cancels, a click outside applies (like the app's other
 * inline editors); a value that doesn't parse is dropped. Anchored where its parent places
 * it (a zero-size box): an automation point (`PointValuePopover`), or the control a value
 * menu was opened on (`ValueEditHost`).
 */

import { useState, type KeyboardEvent } from "react";
import type { ParamInfo } from "@/generated";
import { Popover, TextInput } from "@/kit";
import { editText, editUnit, parseEditText } from "./params";
import { useT } from "@/i18n";

export interface ValueEditPopoverProps {
  /** An open edit (a new object opens it afresh), or null (closed). */
  edit: { plain: number } | null;
  info: ParamInfo;
  /** A plain value (clamped to the info's range). */
  onApply: (plain: number) => void;
  /** Closes it (applied or cancelled). */
  onDone: () => void;
}

export function ValueEditPopover({ edit, info, onApply, onDone }: ValueEditPopoverProps) {
  const t = useT();
  // What it shows, kept through the exit animation.
  const [shown, setShown] = useState<{ edit: { plain: number }; info: ParamInfo; text: string; invalid: boolean } | null>(null);
  if (edit && shown?.edit !== edit) setShown({ edit, info, text: editText(info, edit.plain), invalid: false });
  const cur = edit && shown?.edit === edit ? shown : null;

  const finish = (apply: boolean, keepIfInvalid = false) => {
    if (!cur) return;
    if (apply) {
      const v = parseEditText(cur.info, cur.text);
      if (v === null && keepIfInvalid) {
        setShown({ ...cur, invalid: true });
        return;
      }
      if (v !== null) onApply(v);
    }
    onDone();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    // The surroundings' shortcuts (Delete, arrows, ⌘A...) and the popover's own Escape stay out.
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      finish(true, true);
    } else if (e.key === "Escape") {
      e.preventDefault();
      finish(false);
    }
  };

  const shownInfo = shown?.info ?? info;
  const unit = editUnit(shownInfo);
  return (
    <Popover
      open={edit !== null}
      // Only a click outside closes it from the popover (Escape is handled above): apply.
      onOpenChange={(o) => !o && finish(true)}
      trigger={() => null}
      aria-label={t("automation.value.set", { name: shownInfo.name })}
      className="eth-auto-value-pop"
    >
      {shown && (
        <div className="eth-auto-value-pop__row" onKeyDown={onKeyDown}>
          <TextInput
            size="sm"
            className="eth-auto-value-pop__input"
            aria-label={t("automation.value.field", { name: shownInfo.name })}
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
  );
}
