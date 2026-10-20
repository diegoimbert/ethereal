/**
 * The selection's actions on touch screens (the design's §5), a floating bar at the bottom
 * of the arrangement (no Shift or Ctrl, no keyboard shortcuts):
 *   - clips: select more (taps then toggle, a drag on empty space draws a box), edit, split,
 *     duplicate, copy, paste, nudge one grid step earlier / later, position (the clip sheet:
 *     typed numbers), delete, more (the clip menu);
 *   - a time range (long-press then drag on empty lane space): copy, cut, paste, duplicate,
 *     delete time, loop it, more (the range menu).
 * Also the badge above the finger while a clip is dragged (where it lands, snapped).
 */
import { ChevronLeft, ChevronRight, ClipboardPaste, Copy, CopyPlus, Ellipsis, Hash, ListChecks, PencilLine, Repeat, Scissors, Trash2 } from "lucide-react";
import { useEffect, useReducer, type ReactNode } from "react";
import { useStore } from "zustand";
import { Button, openContextMenu, type ContextMenuEntry } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { formatBarBeat, itemSelection, useTempoMap } from "@/timeline";
import { clipMenu, editTarget, runClipAction, type ClipAction } from "@/features/arrangement/actions";
import { hasClipboard } from "@/features/arrangement/clipboard";
import { selectedClips, sendEdit } from "@/features/arrangement/context";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { runTimeAction, timeSelectionMenu, trackRangeOf, useTimeSelection, type TimeAction } from "@/features/time-edits";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { useTouchDrag } from "./dragState";
import { openClipEditor, useMobileEditor } from "./editorState";
import { nudgeCommand, nudgeStep } from "./nudge";
import { setMultiSelect, useTouchSelect } from "./selectState";
import { useT } from "@/i18n";
import { openSheet } from "./sheets";

function item(label: string, icon: ReactNode, onClick: () => void, opts: { danger?: boolean; disabled?: boolean; active?: boolean } = {}) {
  return (
    <Button
      key={label}
      tone={opts.danger ? "danger" : "ghost"}
      className="eth-clip-actions__btn"
      aria-label={label}
      active={opts.active}
      disabled={opts.disabled}
      onClick={onClick}
    >
      {icon}
      <span className="eth-clip-actions__label">{label}</span>
    </Button>
  );
}

/** The full menu at the bar's right end (the clip or range menu). */
function openMenuAtBar(items: ContextMenuEntry[]): void {
  const bar = document.querySelector(".eth-clip-actions")?.getBoundingClientRect();
  if (!bar) return;
  const noop = () => undefined;
  openContextMenu({ clientX: bar.right, clientY: bar.top, preventDefault: noop, stopPropagation: noop }, items);
}

export function ClipActionBar() {
  const t = useT();
  const transport = useOptionalTransport();
  const count = useStore(itemSelection, (s) => s.selected.clip.size);
  const dragging = useTouchDrag((s) => s.point !== null);
  const editing = useMobileEditor((s) => s.open);
  const multi = useTouchSelect((s) => s.multi);
  const range = useTimeSelection((s) => trackRangeOf(s.selection));
  // The bar acts on the clips only while they are the edit target (a tap on a lane or a header
  // since selected the track: the actions would apply to it).
  useArrangementUi((s) => s.selectedTracks);
  // Re-render when the clipboard may have changed: an edit (cut), or a copy here.
  useProjectStore((s) => s.revision);
  const [, bump] = useReducer((n: number) => n + 1, 0);
  // Select-more ends with the selection (once it stays empty: a selection box clears it for a
  // moment before selecting what it covers).
  useEffect(() => {
    if (count > 0 || !multi) return;
    const timer = setTimeout(() => setMultiSelect(false), 500);
    return () => clearTimeout(timer);
  }, [count, multi]);
  if (!transport || dragging || editing) return null;
  if (range) return <RangeBar onCopied={bump} />;
  if (count === 0 || editTarget() !== "clips") return null;
  const run = (a: ClipAction) => () => void runClipAction(transport, a);
  const nudge = (dir: -1 | 1) => () => {
    const project = useProjectStore.getState().project;
    if (!project) return;
    const beats = dir * nudgeStep(project, useArrangementUi.getState().grid, arrangementView.getState());
    const c = nudgeCommand(project, selectedClips(), beats);
    if (c) void sendEdit(transport, c);
  };
  return (
    <div
      className="eth-clip-actions"
      role="toolbar"
      aria-label={t("mobile.clip_actions.label", { count })}
      data-testid="clip-actions"
      data-multi={multi || undefined}
    >
      {item(multi ? t("mobile.clip_actions.done", { count }) : t("mobile.clip_actions.select_more"), <ListChecks aria-hidden />, () => setMultiSelect(!multi), {
        active: multi,
      })}
      {item(t("mobile.clip_actions.edit"), <PencilLine aria-hidden />, () => {
        const clip = selectedClips()[0];
        if (clip) openClipEditor(clip);
      })}
      {item(t("mobile.clip_actions.split"), <Scissors aria-hidden />, run("split"))}
      {item(t("mobile.clip_actions.duplicate"), <CopyPlus aria-hidden />, run("duplicate"))}
      {item(t("mobile.clip_actions.copy"), <Copy aria-hidden />, () => void runClipAction(transport, "copy").then(bump))}
      {item(t("mobile.clip_actions.paste"), <ClipboardPaste aria-hidden />, run("paste"), { disabled: !hasClipboard() })}
      {item(t("mobile.clip_actions.earlier"), <ChevronLeft aria-hidden />, nudge(-1))}
      {item(t("mobile.clip_actions.later"), <ChevronRight aria-hidden />, nudge(1))}
      {item(
        t("mobile.clip_actions.position"),
        <Hash aria-hidden />,
        () => {
          const clip = selectedClips()[0];
          if (clip) openSheet({ kind: "clip", clip: clip.id });
        },
        { disabled: count !== 1 },
      )}
      {item(t("mobile.clip_actions.delete"), <Trash2 aria-hidden />, run("delete"), { danger: true })}
      {item(t("mobile.clip_actions.more"), <Ellipsis aria-hidden />, () => {
        const clip = selectedClips()[0];
        if (clip) openMenuAtBar(clipMenu(transport, clip));
      })}
    </div>
  );
}

/** A time range's actions. */
function RangeBar({ onCopied }: { onCopied: () => void }) {
  const t = useT();
  const transport = useOptionalTransport();
  const range = useTimeSelection((s) => trackRangeOf(s.selection));
  const hasCopy = useTimeSelection((s) => s.clipboard !== null);
  if (!transport || !range) return null;
  const run = (a: TimeAction) => () => void runTimeAction(transport, a);
  return (
    <div className="eth-clip-actions" role="toolbar" aria-label={t("mobile.clip_actions.time_range")} data-testid="range-actions">
      {item(t("mobile.clip_actions.copy"), <Copy aria-hidden />, () => void runTimeAction(transport, "copy").then(onCopied))}
      {item(t("mobile.clip_actions.cut"), <Scissors aria-hidden />, run("cut"))}
      {item(t("mobile.clip_actions.paste"), <ClipboardPaste aria-hidden />, run("paste"), { disabled: !hasCopy })}
      {item(t("mobile.clip_actions.duplicate"), <CopyPlus aria-hidden />, run("duplicate"))}
      {item(t("mobile.clip_actions.loop_it"), <Repeat aria-hidden />, () => {
        void sendEdit(transport, cmd("Transport", { type: "SetLoopRegion", region: { start: range.start, end: range.end } }));
        void transport.send(cmd("Transport", { type: "SetLoopEnabled", enabled: true }));
      })}
      {item(t("mobile.clip_actions.delete_time"), <Trash2 aria-hidden />, run("delete"), { danger: true })}
      {item(t("mobile.clip_actions.more"), <Ellipsis aria-hidden />, () => openMenuAtBar(timeSelectionMenu(transport)))}
    </div>
  );
}

/** Where the dragged clip lands (snapped), above the finger. */
export function DragBadge() {
  const point = useTouchDrag((s) => s.point);
  const grab = useTouchDrag((s) => s.grab);
  const preview = useArrangementUi((s) => s.preview);
  const tempo = useTempoMap();
  const first = preview ? [...preview.bounds.values()][0] : undefined;
  if (!point || !first) return null;
  const at = grab === "resize-end" ? first.start + first.length : first.start;
  return (
    <output className="eth-drag-badge" data-testid="drag-badge" style={{ left: point.x, top: point.y }}>
      {formatBarBeat(at, tempo)}
    </output>
  );
}
