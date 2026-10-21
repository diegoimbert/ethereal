/**
 * The context menu of every value control (a knob, a fader, a param slider, a number field):
 * "show automation lane" (only for an automatable value), "set value…" (the value popover at
 * the control, like an automation point's) and "reset to default". `openValueMenu` from the
 * control's `onContextMenu`; the app shell mounts `ValueEditHost` once.
 */

import type { MouseEvent } from "react";
import { create } from "zustand";
import type { AutomationTarget, ParamInfo, TrackId } from "@/generated";
import { openContextMenu, type ContextMenuEntry } from "@/kit";
import { t } from "@/i18n";
import { targetKey } from "./params";
import { useAutomationUi } from "./uiStore";
import { ValueEditPopover } from "./ValueEditPopover";
import "./automation.css";

export interface ValueMenuTarget {
  /** The value's name, unit, range and default (the field's text, parsing and reset). */
  info: ParamInfo;
  /** Its current plain value. */
  plain: number;
  /** Sets a plain value (one undo step). */
  set(plain: number): void;
  /** Its automation lane (absent: not automatable, no entry). */
  automation?: { track: TrackId; target: AutomationTarget } | { show(): void };
  /** Reset differently than `set(info.default)` (e.g. a param's own reset). */
  reset?(): void;
}

interface Open {
  target: ValueMenuTarget;
  /** The popover's edit (its starting value). */
  edit: { plain: number };
  /** Viewport point the popover hangs from (the control's bottom-left). */
  x: number;
  y: number;
}

const useValueEdit = create<{ open: Open | null; set(open: Open | null): void }>()((set) => ({
  open: null,
  set: (open) => set({ open }),
}));

/** The value menu's entries for `v` ("set value…" opens at `anchor`). */
export function valueMenuEntries(v: ValueMenuTarget, anchor: Element): ContextMenuEntry[] {
  const out: ContextMenuEntry[] = [];
  const a = v.automation;
  if (a) {
    const show = "show" in a ? a.show : () => useAutomationUi.getState().show(a.track, targetKey(a.target));
    out.push({ label: t("devices.param.show_automation"), onSelect: show }, "separator");
  }
  out.push(
    {
      label: t("automation.menu.set_value"),
      onSelect: () => {
        const r = anchor.getBoundingClientRect();
        useValueEdit.getState().set({ target: v, edit: { plain: v.plain }, x: r.left, y: r.bottom });
      },
    },
    { label: t("devices.param.reset"), disabled: v.plain === v.info.default, onSelect: v.reset ?? (() => v.set(v.info.default)) },
  );
  return out;
}

/** Opens the value menu of the control the event is on. */
export function openValueMenu(e: MouseEvent, v: ValueMenuTarget): void {
  e.preventDefault();
  e.stopPropagation();
  openContextMenu(e, valueMenuEntries(v, e.currentTarget as Element));
}

/** "Set value…" of a value menu: the value popover under its control. Mount once (app shell). */
export function ValueEditHost() {
  const open = useValueEdit((s) => s.open);
  const close = () => useValueEdit.getState().set(null);
  return (
    <div className="eth-value-edit-anchor" style={open ? { left: open.x, top: open.y } : undefined}>
      {open && <ValueEditPopover edit={open.edit} info={open.target.info} onApply={(plain) => open.target.set(plain)} onDone={close} />}
    </div>
  );
}
