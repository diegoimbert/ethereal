/**
 * The clip sheet (the design's §6 "Numeric entry"): a clip's position typed in, where a
 * finger is too coarse. Start ("bar.beat.sixteenth") and length ("bars.beats.sixteenths"), and
 * each edge nudged by one grid step. Opened from the clip bar's "position". Each change is one
 * undo step, through the same commands as a drag (`editMath.ts`).
 */
import { useEffect } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { Beats, Clip, Command } from "@/generated";
import { Button } from "@/kit";
import { clipDisplayName, defaultClipName, useProjectStore } from "@/state";
import { beatsPerBar, formatBarBeat, formatDuration, useTempoMap } from "@/timeline";
import { newId } from "@/transport";
import { sendEdit } from "@/features/arrangement/context";
import { boundsCommand, dragPreview, moveCommand, type DragMode } from "@/features/arrangement/editMath";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { nudgeStep } from "./nudge";
import { parseLength, parsePosition } from "./position";
import { closeSheet, useMobileSheets } from "./sheets";
import { Sheet } from "./Sheet";
import { useT } from "@/i18n";

export function ClipSheetRoot() {
  const t = useT();
  const sheet = useMobileSheets((s) => s.sheet);
  const clip = useProjectStore((s) => (sheet?.kind === "clip" ? (s.project?.clips[sheet.clip] ?? null) : null));
  // The clip went away (deleted, undone): the sheet closes.
  useEffect(() => {
    if (sheet?.kind === "clip" && !clip) closeSheet();
  }, [sheet, clip]);
  const name = useProjectStore((s) => (clip && s.project ? clip.name || defaultClipName(s.project, clip) || clipDisplayName(s.project, clip) : ""));
  const title = (
    <span className="eth-track-sheet__title">
      {t("mobile.clip_sheet.title")}
      {name && <span className="eth-user-text">· {name}</span>}
    </span>
  );
  return (
    <Sheet open={!!clip} onClose={closeSheet} label={t("mobile.clip_sheet.label")} title={title} testId="clip-sheet">
      {clip && <ClipSheetBody clip={clip} />}
    </Sheet>
  );
}

/** The command moving `clip` (`mode`) by `delta` beats, unsnapped. */
function edit(clip: Clip, mode: DragMode, delta: Beats): Command | null {
  const preview = dragPreview({ clips: [clip], anchor: clip.id, rows: [], snap: (b) => b }, mode, delta, 0);
  if (mode !== "move") return boundsCommand([clip], preview);
  const all = Object.values(useProjectStore.getState().project?.clips ?? {});
  return moveCommand([clip], preview, false, newId, all);
}

function ClipSheetBody({ clip }: { clip: Clip }) {
  const t = useT();
  const transport = useOptionalTransport();
  const tempo = useTempoMap();
  if (!transport) return null;
  /** Sends `c`; false when there is nothing to change. */
  const send = (c: Command | null): boolean => {
    if (c) void sendEdit(transport, c);
    return c !== null;
  };
  const barLength = beatsPerBar(tempo.signatureAt(clip.start));
  const start = formatBarBeat(clip.start, tempo);
  const length = formatDuration(clip.length, barLength);
  const step = () => {
    const project = useProjectStore.getState().project;
    return project ? nudgeStep(project, useArrangementUi.getState().grid, arrangementView.getState()) : 1;
  };
  const nudge = (mode: DragMode, dir: -1 | 1) => () => void send(edit(clip, mode, dir * step()));

  return (
    <div className="eth-track-sheet eth-clip-sheet">
      <Field
        label={t("mobile.clip_sheet.start")}
        ariaLabel={t("mobile.clip_sheet.start_aria")}
        value={start}
        commit={(text) => {
          const at = parsePosition(text, tempo);
          return at !== null && send(edit(clip, "move", at - clip.start));
        }}
      />
      <Field
        label={t("mobile.clip_sheet.length")}
        ariaLabel={t("mobile.clip_sheet.length_aria")}
        value={length}
        commit={(text) => {
          const l = parseLength(text, barLength);
          return l !== null && l > 0 && send(edit(clip, "resize-end", l - clip.length));
        }}
      />
      <div className="eth-track-sheet__row">
        <span className="eth-track-sheet__label">{t("mobile.clip_sheet.start_edge")}</span>
        <Nudge label={t("mobile.clip_sheet.start_edge")} onEarlier={nudge("resize-start", -1)} onLater={nudge("resize-start", 1)} />
      </div>
      <div className="eth-track-sheet__row">
        <span className="eth-track-sheet__label">{t("mobile.clip_sheet.end_edge")}</span>
        <Nudge label={t("mobile.clip_sheet.end_edge")} onEarlier={nudge("resize-end", -1)} onLater={nudge("resize-end", 1)} />
      </div>
      <p className="eth-track-sheet__hint">{t("mobile.clip_sheet.hint")}</p>
    </div>
  );
}

/** A typed value: committed on Enter or when the field loses focus; an invalid entry goes back. */
function Field({ label, ariaLabel, value, commit }: { label: string; ariaLabel: string; value: string; commit(text: string): boolean }) {
  return (
    <label className="eth-track-sheet__row">
      <span className="eth-track-sheet__label">{label}</span>
      <input
        // A change elsewhere (a drag, undo, a peer) shows here.
        key={value}
        className="eth-track-sheet__name"
        aria-label={ariaLabel}
        inputMode="decimal"
        autoComplete="off"
        defaultValue={value}
        onBlur={(e) => {
          const text = e.currentTarget.value;
          if (text.trim() === value || !commit(text)) e.currentTarget.value = value;
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
        }}
      />
    </label>
  );
}

function Nudge({ label, onEarlier, onLater }: { label: string; onEarlier(): void; onLater(): void }) {
  const t = useT();
  return (
    <span className="eth-track-sheet__actions eth-clip-sheet__nudge" role="group" aria-label={label}>
      <Button aria-label={t("mobile.clip_sheet.edge_earlier", { edge: label })} onClick={onEarlier}>
        <ChevronLeft aria-hidden /> {t("mobile.clip_sheet.earlier")}
      </Button>
      <Button aria-label={t("mobile.clip_sheet.edge_later", { edge: label })} onClick={onLater}>
        {t("mobile.clip_sheet.later")} <ChevronRight aria-hidden />
      </Button>
    </span>
  );
}
