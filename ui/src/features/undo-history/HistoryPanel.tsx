import clsx from "clsx";
import { useContributions } from "@/ext";
import { Flag } from "lucide-react";
import { useEffect, useRef, type KeyboardEvent } from "react";
import type { HistoryList, HistoryStep } from "@/generated";
import { IconButton, openContextMenu, TextInput } from "@/kit";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { cmd, type EngineTransport } from "@/transport";
import { MAX_CHECKPOINT_CHARS, stepTime } from "./format";
import { useUndoHistoryUi } from "./store";
import { useHistoryList } from "./useHistoryList";
import { formatDateTime, t, tText, useT } from "@/i18n";

/** The step's label in the user's language (the engine's English when it has no message). */
function stepLabel(step: HistoryStep): string {
  return tText(step.label_key, step.label);
}

function stepTitle(step: HistoryStep): string {
  const when = formatDateTime(step.time_ms, { dateStyle: "medium", timeStyle: "medium" });
  const what = step.checkpoint ? `${step.checkpoint} — ${stepLabel(step)}` : stepLabel(step);
  return `${what}\n${when}${step.undone ? `\n${t("history.undone_hint")}` : ""}`;
}

/** Move focus between the rows' jump buttons with the arrow keys. */
function onListKey(e: KeyboardEvent<HTMLOListElement>) {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
  const buttons = [...e.currentTarget.querySelectorAll<HTMLButtonElement>(".eth-history__jump")];
  if (!buttons.length) return;
  const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
  const next = e.key === "Home" ? 0 : e.key === "End" ? buttons.length - 1 : Math.max(0, Math.min(buttons.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)));
  e.preventDefault();
  buttons[next]?.focus();
}

function CheckpointEditor({ step, onDone }: { step: HistoryStep; onDone: (name: string | null | undefined) => void }) {
  const t = useT();
  const done = useRef(false);
  const finish = (name: string | null | undefined) => {
    if (done.current) return;
    done.current = true;
    onDone(name);
  };
  return (
    <TextInput
      size="sm"
      className="eth-history__name-input"
      aria-label={t("history.checkpoint_name_for", { step: stepLabel(step) })}
      placeholder={t("history.checkpoint_name")}
      defaultValue={step.checkpoint ?? ""}
      maxLength={MAX_CHECKPOINT_CHARS}
      autoFocus
      onFocus={(e) => e.currentTarget.select()}
      onKeyDown={(e) => {
        if (e.key === "Enter") finish(e.currentTarget.value.trim() || null);
        else if (e.key === "Escape") {
          e.stopPropagation();
          finish(undefined);
        }
      }}
      onBlur={(e) => finish(e.currentTarget.value.trim() || null)}
    />
  );
}

function StepRow({
  step,
  current,
  editing,
  onJump,
  onEdit,
  onName,
}: {
  step: HistoryStep;
  current: boolean;
  editing: boolean;
  onJump: () => void;
  onEdit: () => void;
  /** `undefined` = cancelled. */
  onName: (name: string | null | undefined) => void;
}) {
  const t = useT();
  const ref = useRef<HTMLLIElement>(null);
  useEffect(() => {
    if (current) ref.current?.scrollIntoView?.({ block: "nearest" });
  }, [current]);
  return (
    <li
      ref={ref}
      className={clsx(
        "eth-history__row",
        current && "eth-history__row--current",
        step.undone && "eth-history__row--undone",
        step.checkpoint && "eth-history__row--checkpoint",
      )}
      data-step={step.id}
      onContextMenu={(e) =>
        openContextMenu(e, [
          { label: step.undone ? t("history.redo_to_here") : t("history.undo_to_here"), disabled: current, onSelect: onJump },
          { label: step.checkpoint ? t("history.rename_checkpoint_menu") : t("history.name_checkpoint_menu"), onSelect: onEdit },
          ...(step.checkpoint ? [{ label: t("history.remove_checkpoint"), onSelect: () => onName(null) }] : []),
        ])
      }
    >
      <span className="eth-history__dot" aria-hidden />
      {editing ? (
        <div className="eth-history__edit">
          <CheckpointEditor step={step} onDone={onName} />
        </div>
      ) : (
        <>
          <button type="button" className="eth-history__jump" aria-current={current ? "step" : undefined} title={stepTitle(step)} onClick={onJump}>
            <span className="eth-history__text">
              {step.checkpoint && <span className="eth-history__checkpoint eth-user-text">{step.checkpoint}</span>}
              <span className="eth-history__label">{stepLabel(step)}</span>
            </span>
            <time className="eth-history__time" dateTime={new Date(step.time_ms).toISOString()}>
              {stepTime(step.time_ms)}
            </time>
          </button>
          <IconButton
            size="sm"
            tone="ghost"
            className="eth-history__flag"
            label={step.checkpoint ? t("history.rename_checkpoint") : t("history.name_checkpoint")}
            active={!!step.checkpoint}
            icon={<Flag />}
            onClick={onEdit}
          />
        </>
      )}
    </li>
  );
}

function OriginRow({ list, onJump }: { list: HistoryList; onJump: () => void }) {
  const t = useT();
  const current = list.current === null;
  return (
    <li className={clsx("eth-history__row", "eth-history__row--origin", current && "eth-history__row--current")} data-step="origin">
      <span className="eth-history__dot" aria-hidden />
      <button
        type="button"
        className="eth-history__jump"
        aria-current={current ? "step" : undefined}
        title={list.truncated ? t("history.undo_all_kept") : t("history.undo_all")}
        onClick={onJump}
      >
        <span className="eth-history__text">
          <span className="eth-history__label">{list.truncated ? t("history.oldest_kept") : t("history.project_opened")}</span>
        </span>
      </button>
    </li>
  );
}

/** Undo history (left rail tab "History"): click a step to go back (or forward) to it. */
export function HistoryPanel() {
  const t = useT();
  const transport = useOptionalTransport();
  if (!transport) return <p className="eth-history__hint">{t("history.no_engine")}</p>;
  return <HistoryPanelBody transport={transport} />;
}

function HistoryPanelBody({ transport }: { transport: EngineTransport }) {
  const hints = useContributions("history.hint");
  const t = useT();
  const { list, setList, error, setError } = useHistoryList(transport);
  const editing = useUndoHistoryUi((s) => s.editing);
  const editCurrent = useUndoHistoryUi((s) => s.editCurrent);
  const setEditing = useUndoHistoryUi((s) => s.setEditing);

  // Palette "Name checkpoint…": edit the current step once the list is here.
  useEffect(() => {
    if (!editCurrent || !list) return;
    setEditing(list.current);
  }, [editCurrent, list, setEditing]);

  const report = (e: unknown) => setError(e instanceof Error ? e.message : String(e));

  const jump = (step: number | null) => {
    if (list && list.current === step) return;
    transport.send(cmd("History", { type: "JumpTo", step })).then((r) => {
      if (r.type === "History") setList(r.history);
      setError(null);
    }, report);
  };

  const name = (step: number, value: string | null | undefined) => {
    setEditing(null);
    if (value === undefined) return;
    const prev = list?.steps.find((s) => s.id === step)?.checkpoint ?? null;
    if (prev === value) return;
    transport.send(cmd("History", { type: "SetCheckpoint", step, name: value })).then(() => {
      // The engine pushes the new list; show it at once.
      setList((l) => l && { ...l, steps: l.steps.map((s) => (s.id === step ? { ...s, checkpoint: value } : s)) });
      setError(null);
    }, report);
  };

  if (!list) {
    return (
      <div className="eth-history" data-feature="undo-history" aria-busy={!error}>
        {error && <p className="eth-history__error">{error}</p>}
      </div>
    );
  }

  const undone = list.steps.filter((s) => s.undone).length;
  return (
    <div className="eth-history" data-feature="undo-history">
      {/* Extensions' notes about the history (`@/ext` "history.hint"). */}
      {hints.map((Hint, i) => (
        <Hint key={i} />
      ))}
      {list.truncated && <p className="eth-history__hint">{t("history.truncated")}</p>}
      <ol className="eth-history__list" aria-label={t("history.list")} onKeyDown={onListKey}>
        <OriginRow list={list} onJump={() => jump(null)} />
        {list.steps.map((s) => (
          <StepRow
            key={s.id}
            step={s}
            current={list.current === s.id}
            editing={editing === s.id}
            onJump={() => jump(s.id)}
            onEdit={() => setEditing(s.id)}
            onName={(v) => name(s.id, v)}
          />
        ))}
      </ol>
      {list.steps.length === 0 && <p className="eth-history__empty">{t("history.empty")}</p>}
      {error && <p className="eth-history__error">{error}</p>}
      <footer className="eth-history__footer">
        <span>{t("history.count", { steps: list.steps.length, undone })}</span>
        <span className="eth-history__footer-note">{t("history.not_saved")}</span>
      </footer>
    </div>
  );
}
