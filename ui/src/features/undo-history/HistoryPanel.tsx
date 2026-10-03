import clsx from "clsx";
import { Flag } from "lucide-react";
import { useEffect, useRef, type KeyboardEvent } from "react";
import type { HistoryList, HistoryStep } from "@/generated";
import { IconButton, openContextMenu, TextInput } from "@/kit";
import { useCollabStore } from "@/features/collab/store";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { cmd, type EngineTransport } from "@/transport";
import { useUndoHistoryUi } from "./store";
import { useHistoryList } from "./useHistoryList";

/** Longest checkpoint name the engine keeps. */
export const MAX_CHECKPOINT_CHARS = 120;

/** Clock time of a step ("14:03:12"); the full date is in the row's tooltip. */
export function stepTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23" });
}

function stepTitle(step: HistoryStep): string {
  const when = new Date(step.time_ms).toLocaleString();
  const what = step.checkpoint ? `${step.checkpoint} — ${step.label}` : step.label;
  return `${what}\n${when}${step.undone ? "\nUndone: click to redo up to here" : ""}`;
}

/** Move focus between the rows' jump buttons with the arrow keys. */
function onListKey(e: KeyboardEvent<HTMLOListElement>) {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Home" && e.key !== "End") return;
  const buttons = [...e.currentTarget.querySelectorAll<HTMLButtonElement>(".eth-history__jump")];
  if (!buttons.length) return;
  const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
  const next =
    e.key === "Home"
      ? 0
      : e.key === "End"
        ? buttons.length - 1
        : Math.max(0, Math.min(buttons.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)));
  e.preventDefault();
  buttons[next]?.focus();
}

function CheckpointEditor({ step, onDone }: { step: HistoryStep; onDone: (name: string | null | undefined) => void }) {
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
      aria-label={`Checkpoint name for ${step.label}`}
      placeholder="Checkpoint name"
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
          { label: step.undone ? "Redo to here" : "Undo to here", disabled: current, onSelect: onJump },
          { label: step.checkpoint ? "Rename checkpoint…" : "Name checkpoint…", onSelect: onEdit },
          ...(step.checkpoint ? [{ label: "Remove checkpoint", onSelect: () => onName(null) }] : []),
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
          <button
            type="button"
            className="eth-history__jump"
            aria-current={current ? "step" : undefined}
            title={stepTitle(step)}
            onClick={onJump}
          >
            <span className="eth-history__text">
              {step.checkpoint && <span className="eth-history__checkpoint">{step.checkpoint}</span>}
              <span className="eth-history__label">{step.label}</span>
            </span>
            <time className="eth-history__time" dateTime={new Date(step.time_ms).toISOString()}>
              {stepTime(step.time_ms)}
            </time>
          </button>
          <IconButton
            size="sm"
            tone="ghost"
            className="eth-history__flag"
            label={step.checkpoint ? "Rename checkpoint" : "Name checkpoint"}
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
  const current = list.current === null;
  return (
    <li
      className={clsx(
        "eth-history__row",
        "eth-history__row--origin",
        current && "eth-history__row--current",
      )}
      data-step="origin"
    >
      <span className="eth-history__dot" aria-hidden />
      <button
        type="button"
        className="eth-history__jump"
        aria-current={current ? "step" : undefined}
        title={list.truncated ? "Undo every step still in the history" : "Undo everything since the project was opened"}
        onClick={onJump}
      >
        <span className="eth-history__text">
          <span className="eth-history__label">{list.truncated ? "Oldest kept state" : "Project opened"}</span>
        </span>
      </button>
    </li>
  );
}

/** Undo history (left rail tab "History"): click a step to go back (or forward) to it. */
export function HistoryPanel() {
  const transport = useOptionalTransport();
  if (!transport) return <p className="eth-history__hint">No engine connected</p>;
  return <HistoryPanelBody transport={transport} />;
}

function HistoryPanelBody({ transport }: { transport: EngineTransport }) {
  const { list, setList, error, setError } = useHistoryList(transport);
  const inSession = useCollabStore((s) => s.status.type === "Online");
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
      {inSession && (
        <p className="eth-history__hint">Only your own edits are listed. Going back keeps what others changed since.</p>
      )}
      {list.truncated && <p className="eth-history__hint">Older steps were dropped from the history.</p>}
      <ol className="eth-history__list" aria-label="Undo history" onKeyDown={onListKey}>
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
      {list.steps.length === 0 && <p className="eth-history__empty">Edits you make appear here.</p>}
      {error && <p className="eth-history__error">{error}</p>}
      <footer className="eth-history__footer">
        <span>
          {list.steps.length} {list.steps.length === 1 ? "step" : "steps"}
          {undone > 0 && ` · ${undone} undone`}
        </span>
        <span className="eth-history__footer-note">Not saved with the project</span>
      </footer>
    </div>
  );
}
