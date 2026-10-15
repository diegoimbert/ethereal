import clsx from "clsx";
import { AlertTriangle, Plus, Printer, RotateCcw, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { KeymapPreset } from "@/generated";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { Badge, Button, Dialog, IconButton, Select, TextInput, Toggle } from "@/kit";
import { eventChords, formatChord, MAX_CHORDS_PER_ACTION } from "./chords";
import { findConflicts, conflictsFor } from "./conflicts";
import { pauseDispatch } from "./dispatcher";
import { derivePaletteActions } from "./palette";
import { printCheatSheet } from "./print";
import { Trans, foldForSearch, formatList, inEnglish, useT } from "@/i18n";
import { actionLabel, groupActions, groupLabel, scopesLabel, useActions } from "./grouping";
import type { KeymapAction } from "./registry";
import { closeKeymapEditor, effectiveChords, presetChords, PRESET_LABELS, resetKeymap, saveKeymap, useKeymapStore, withBinding, withPreset } from "./store";

const PRESETS: ReadonlyArray<KeymapPreset> = ["Ethereal", "AbletonLike"];

/** Several actions' names, as a list ("undo and copy"). */
const actionList = (actions: ReadonlyArray<KeymapAction>) => formatList(actions.map(actionLabel));

/**
 * Keyboard shortcuts: pick a preset, search, rebind any action (record a chord, remove one,
 * reset the row), see conflicts, print a cheat sheet. Every change applies at once and is
 * stored by the engine in the user library (`Keymap::Set`).
 */
export function KeymapEditor() {
  const t = useT();
  const open = useKeymapStore((s) => s.editorOpen);
  return (
    <Dialog
      open={open}
      onClose={closeKeymapEditor}
      title={t("shell.keymap.title")}
      className="eth-keymap"
      footer={
        <>
          <Button tone="ghost" onClick={() => printCheatSheet()}>
            <Printer aria-hidden /> {t("shell.keymap.print")}
          </Button>
          <Button onClick={closeKeymapEditor}>{t("common.done")}</Button>
        </>
      }
    >
      {open && <EditorBody />}
    </Dialog>
  );
}

function EditorBody() {
  const t = useT();
  const transport = useOptionalTransport();
  const keymap = useKeymapStore((s) => s.keymap);
  const stored = useKeymapStore((s) => s.stored);
  const error = useKeymapStore((s) => s.error);
  const [query, setQuery] = useState("");
  const [onlyConflicts, setOnlyConflicts] = useState(false);
  const [recording, setRecording] = useState<string | null>(null);

  // The palette's commands (incl. ones other features add) are actions too.
  useEffect(() => void derivePaletteActions(), []);
  const actions = useActions();
  const conflicts = useMemo(() => findConflicts(actions, keymap), [actions, keymap]);

  const q = foldForSearch(query.trim());
  const shown = actions.filter((a) => {
    if (onlyConflicts && !conflicts.has(a.id)) return false;
    if (!q) return true;
    const chords = effectiveChords(a, keymap).map((c) => `${c} ${formatChord(c)}`);
    // The label in the shown language and in English (the docs and habits name it in English).
    const english = inEnglish(() => actionLabel(a));
    return foldForSearch(`${actionLabel(a)} ${english} ${groupLabel(a.group)} ${a.keywords ?? ""} ${chords.join(" ")}`).includes(q);
  });

  const setChords = (id: string, chords: string[] | null) => void saveKeymap(transport, withBinding(useKeymapStore.getState().keymap, id, chords));

  return (
    <div className="eth-keymap__body" data-testid="keymap-editor">
      <div className="eth-keymap__toolbar">
        <label className="eth-keymap__preset">
          <span className="eth-keymap__dim">{t("shell.keymap.preset")}</span>
          <Select
            size="sm"
            aria-label={t("shell.keymap.preset_label")}
            options={PRESETS.map((value) => ({ value, label: t(PRESET_LABELS[value]) }))}
            value={keymap.preset}
            onChange={(p) => void saveKeymap(transport, withPreset(useKeymapStore.getState().keymap, p))}
          />
        </label>
        <TextInput
          size="sm"
          className="eth-keymap__search"
          placeholder={t("shell.keymap.search_placeholder")}
          aria-label={t("shell.keymap.search_label")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <Toggle size="sm" checked={onlyConflicts} onChange={setOnlyConflicts} label={<ConflictCount count={conflicts.size} />} />
      </div>
      {!stored && <p className="eth-keymap__note">{t("shell.keymap.not_stored")}</p>}
      {error && (
        <p className="eth-keymap__error" role="alert">
          {t("shell.keymap.save_failed", { error })}
        </p>
      )}
      <div className="eth-keymap__list" role="list" aria-label={t("shell.keymap.list")}>
        {shown.length === 0 && <p className="eth-keymap__note">{t("shell.keymap.no_match")}</p>}
        {groupActions(shown).map(({ group, actions: rows }) => (
          <section key={group} className="eth-keymap__group" aria-label={groupLabel(group)}>
            <h3 className="eth-keymap__heading">{groupLabel(group)}</h3>
            {rows.map((a) => (
              <Row
                key={a.id}
                action={a}
                chords={effectiveChords(a, keymap)}
                overridden={keymap.overrides.some((o) => o.action === a.id)}
                presetChords={presetChords(a, keymap.preset)}
                conflicts={conflicts.get(a.id) ?? []}
                recording={recording === a.id}
                onRecord={() => setRecording(a.id)}
                onRecorded={(chord) => {
                  setRecording(null);
                  const now = effectiveChords(a, useKeymapStore.getState().keymap);
                  if (chord && !now.includes(chord) && now.length < MAX_CHORDS_PER_ACTION) setChords(a.id, [...now, chord]);
                }}
                onRemove={(chord) =>
                  setChords(
                    a.id,
                    effectiveChords(a, useKeymapStore.getState().keymap).filter((c) => c !== chord),
                  )
                }
                onReset={() => setChords(a.id, null)}
                onUnbindOther={(other, chord) => {
                  const theirs = effectiveChords(other, useKeymapStore.getState().keymap);
                  setChords(
                    other.id,
                    theirs.filter((c) => c !== chord && formatChord(c) !== formatChord(chord)),
                  );
                }}
              />
            ))}
          </section>
        ))}
      </div>
      <div className="eth-keymap__actions">
        <Button size="sm" tone="ghost" disabled={keymap.overrides.length === 0 && keymap.preset === "Ethereal"} onClick={() => void resetKeymap(transport)}>
          <RotateCcw aria-hidden /> {t("shell.keymap.reset_all")}
        </Button>
      </div>
    </div>
  );
}

function ConflictCount({ count }: { count: number }) {
  const t = useT();
  return (
    <span className="eth-keymap__conflict-toggle">
      {t("shell.keymap.conflicts_only")} {count > 0 && <Badge tone="warn">{count}</Badge>}
    </span>
  );
}

interface RowProps {
  action: KeymapAction;
  chords: string[];
  presetChords: string[];
  overridden: boolean;
  conflicts: ReadonlyArray<{ chord: string; with: KeymapAction[] }>;
  recording: boolean;
  onRecord(): void;
  onRecorded(chord: string | null): void;
  onRemove(chord: string): void;
  onReset(): void;
  onUnbindOther(other: KeymapAction, chord: string): void;
}

function Row({ action, chords, presetChords, overridden, conflicts, recording, onRecord, onRecorded, onRemove, onReset, onUnbindOther }: RowProps) {
  const t = useT();
  const keymap = useKeymapStore((s) => s.keymap);
  const actions = useActions();
  const scope = scopesLabel(action);
  const name = actionLabel(action);
  return (
    <div className={clsx("eth-keymap__row", conflicts.length > 0 && "eth-keymap__row--conflict")} role="listitem" data-action={action.id}>
      <div className="eth-keymap__label">
        <span>{name}</span>
        {scope && <span className="eth-keymap__dim">{scope}</span>}
      </div>
      <div className="eth-keymap__chords">
        {chords.length === 0 && !recording && <span className="eth-keymap__dim">{t("shell.keymap.not_set")}</span>}
        {chords.map((c) => (
          <span key={c} className="eth-keymap__chord" data-chord={c}>
            <kbd className="eth-keymap__kbd">{formatChord(c)}</kbd>
            <IconButton
              size="sm"
              tone="ghost"
              label={t("shell.keymap.remove_chord", { chord: formatChord(c), action: name })}
              icon={<X />}
              onClick={() => onRemove(c)}
            />
          </span>
        ))}
        {recording ? (
          <Recorder action={action} onDone={onRecorded} keymapChords={(chord) => conflictsFor(action, chord, actions, keymap)} />
        ) : (
          chords.length < MAX_CHORDS_PER_ACTION && (
            <IconButton size="sm" tone="ghost" label={t("shell.keymap.add_chord", { action: name })} icon={<Plus />} onClick={onRecord} />
          )
        )}
        {overridden && (
          <IconButton
            size="sm"
            tone="ghost"
            label={
              presetChords.length
                ? t("shell.keymap.reset_row", { action: name, chords: presetChords.map((c) => formatChord(c)).join(", ") })
                : t("shell.keymap.reset_row_none", { action: name })
            }
            icon={<RotateCcw />}
            onClick={onReset}
          />
        )}
      </div>
      {conflicts.map((c) => (
        <div key={c.chord} className="eth-keymap__conflict" role="note">
          <AlertTriangle aria-hidden className="eth-keymap__warn-icon" />
          <span>
            <Trans k="shell.keymap.conflict" values={{ chord: <kbd className="eth-keymap__kbd">{formatChord(c.chord)}</kbd>, actions: actionList(c.with) }} />
          </span>
          {c.with.map((o) => (
            <Button key={o.id} size="sm" tone="ghost" onClick={() => onUnbindOther(o, c.chord)}>
              {t("shell.keymap.remove_from", { action: actionLabel(o) })}
            </Button>
          ))}
        </div>
      ))}
    </div>
  );
}

/** "Press a shortcut…": records the next chord (Escape cancels). */
function Recorder({ action, onDone, keymapChords }: { action: KeymapAction; onDone(chord: string | null): void; keymapChords(chord: string): KeymapAction[] }) {
  const t = useT();
  const [pending, setPending] = useState<string | null>(null);
  const latest = useRef({ onDone, pending });
  useEffect(() => {
    latest.current = { onDone, pending };
  });
  useEffect(() => {
    const resume = pauseDispatch();
    const onKey = (e: KeyboardEvent) => {
      const [chord] = eventChords(e);
      if (!chord) return; // a lone modifier: keep waiting
      e.preventDefault();
      e.stopPropagation();
      const { onDone, pending } = latest.current;
      if (chord === "Escape") onDone(null);
      // Enter confirms a recorded chord (a first Enter records Enter itself).
      else if (chord === "Enter" && pending) onDone(pending);
      else setPending(chord);
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => {
      resume();
      window.removeEventListener("keydown", onKey, { capture: true });
    };
  }, []);
  if (pending) {
    const clash = keymapChords(pending);
    return (
      <span className="eth-keymap__pending" data-testid="keymap-pending">
        <kbd className="eth-keymap__kbd eth-keymap__kbd--recording">{formatChord(pending)}</kbd>
        {clash.length > 0 && (
          <span className="eth-keymap__clash" role="alert">
            <AlertTriangle aria-hidden className="eth-keymap__warn-icon" /> {t("shell.keymap.clash", { actions: actionList(clash) })}
          </span>
        )}
        <Button size="sm" tone="accent" onClick={() => onDone(pending)}>
          {clash.length > 0 ? t("shell.keymap.use_anyway") : t("shell.keymap.use")}
        </Button>
        <Button size="sm" tone="ghost" onClick={() => onDone(null)}>
          {t("shell.keymap.cancel")}
        </Button>
      </span>
    );
  }
  return (
    <span className="eth-keymap__pending" data-testid="keymap-recording">
      <kbd className="eth-keymap__kbd eth-keymap__kbd--recording" aria-live="polite">
        {t("shell.keymap.recording", { action: actionLabel(action) })}
      </kbd>
      <Button size="sm" tone="ghost" onClick={() => onDone(null)}>
        {t("shell.keymap.cancel")}
      </Button>
    </span>
  );
}
