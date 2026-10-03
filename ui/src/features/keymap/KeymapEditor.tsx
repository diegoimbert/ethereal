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
import { groupActions, useActions } from "./grouping";
import { SCOPE_LABELS, type KeymapAction } from "./registry";
import {
  closeKeymapEditor,
  effectiveChords,
  presetChords,
  PRESET_LABELS,
  resetKeymap,
  saveKeymap,
  useKeymapStore,
  withBinding,
  withPreset,
} from "./store";

const PRESETS: ReadonlyArray<{ value: KeymapPreset; label: string }> = [
  { value: "Ethereal", label: PRESET_LABELS.Ethereal },
  { value: "AbletonLike", label: PRESET_LABELS.AbletonLike },
];

/**
 * Keyboard shortcuts: pick a preset, search, rebind any action (record a chord, remove one,
 * reset the row), see conflicts, print a cheat sheet. Every change applies at once and is
 * stored by the engine in the user library (`Keymap::Set`).
 */
export function KeymapEditor() {
  const open = useKeymapStore((s) => s.editorOpen);
  return (
    <Dialog
      open={open}
      onClose={closeKeymapEditor}
      title="Keyboard shortcuts"
      className="eth-keymap"
      footer={
        <>
          <Button tone="ghost" onClick={() => printCheatSheet()}>
            <Printer aria-hidden /> Print cheat sheet
          </Button>
          <Button onClick={closeKeymapEditor}>Done</Button>
        </>
      }
    >
      {open && <EditorBody />}
    </Dialog>
  );
}

function EditorBody() {
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

  const q = query.trim().toLowerCase();
  const shown = actions.filter((a) => {
    if (onlyConflicts && !conflicts.has(a.id)) return false;
    if (!q) return true;
    const chords = effectiveChords(a, keymap).map((c) => `${c} ${formatChord(c)}`);
    return `${a.label} ${a.group} ${a.keywords ?? ""} ${chords.join(" ")}`.toLowerCase().includes(q);
  });

  const setChords = (id: string, chords: string[] | null) => void saveKeymap(transport, withBinding(useKeymapStore.getState().keymap, id, chords));

  return (
    <div className="eth-keymap__body" data-testid="keymap-editor">
      <div className="eth-keymap__toolbar">
        <label className="eth-keymap__preset">
          <span className="eth-keymap__dim">Preset</span>
          <Select
            size="sm"
            aria-label="Keymap preset"
            options={PRESETS}
            value={keymap.preset}
            onChange={(p) => void saveKeymap(transport, withPreset(useKeymapStore.getState().keymap, p))}
          />
        </label>
        <TextInput
          size="sm"
          className="eth-keymap__search"
          placeholder="Search actions or keys…"
          aria-label="Search shortcuts"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <Toggle size="sm" checked={onlyConflicts} onChange={setOnlyConflicts} label={<ConflictCount count={conflicts.size} />} />
      </div>
      {!stored && <p className="eth-keymap__note">This engine can't store shortcuts: changes last until you close Ethereal.</p>}
      {error && (
        <p className="eth-keymap__error" role="alert">
          Couldn't save: {error}
        </p>
      )}
      <div className="eth-keymap__list" role="list" aria-label="Shortcuts">
        {shown.length === 0 && <p className="eth-keymap__note">No matching action.</p>}
        {groupActions(shown).map(({ group, actions: rows }) => (
          <section key={group} className="eth-keymap__group" aria-label={group}>
            <h3 className="eth-keymap__heading">{group}</h3>
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
                onRemove={(chord) => setChords(a.id, effectiveChords(a, useKeymapStore.getState().keymap).filter((c) => c !== chord))}
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
        <Button
          size="sm"
          tone="ghost"
          disabled={keymap.overrides.length === 0 && keymap.preset === "Ethereal"}
          onClick={() => void resetKeymap(transport)}
        >
          <RotateCcw aria-hidden /> Reset all to Ethereal defaults
        </Button>
      </div>
    </div>
  );
}

function ConflictCount({ count }: { count: number }) {
  return (
    <span className="eth-keymap__conflict-toggle">
      Conflicts only {count > 0 && <Badge tone="warn">{count}</Badge>}
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
  const keymap = useKeymapStore((s) => s.keymap);
  const actions = useActions();
  const scope = action.scopes.includes("global") ? null : action.scopes.map((s) => SCOPE_LABELS[s]).join(", ");
  return (
    <div className={clsx("eth-keymap__row", conflicts.length > 0 && "eth-keymap__row--conflict")} role="listitem" data-action={action.id}>
      <div className="eth-keymap__label">
        <span>{action.label}</span>
        {scope && <span className="eth-keymap__dim">{scope}</span>}
      </div>
      <div className="eth-keymap__chords">
        {chords.length === 0 && !recording && <span className="eth-keymap__dim">Not set</span>}
        {chords.map((c) => (
          <span key={c} className="eth-keymap__chord" data-chord={c}>
            <kbd className="eth-keymap__kbd">{formatChord(c)}</kbd>
            <IconButton size="sm" tone="ghost" label={`Remove ${formatChord(c)} from ${action.label}`} icon={<X />} onClick={() => onRemove(c)} />
          </span>
        ))}
        {recording ? (
          <Recorder action={action} onDone={onRecorded} keymapChords={(chord) => conflictsFor(action, chord, actions, keymap)} />
        ) : (
          chords.length < MAX_CHORDS_PER_ACTION && (
            <IconButton size="sm" tone="ghost" label={`Add a shortcut for ${action.label}`} icon={<Plus />} onClick={onRecord} />
          )
        )}
        {overridden && (
          <IconButton
            size="sm"
            tone="ghost"
            label={`Reset ${action.label} to ${presetChords.length ? presetChords.map((c) => formatChord(c)).join(", ") : "no shortcut"}`}
            icon={<RotateCcw />}
            onClick={onReset}
          />
        )}
      </div>
      {conflicts.map((c) => (
        <div key={c.chord} className="eth-keymap__conflict" role="note">
          <AlertTriangle aria-hidden className="eth-keymap__warn-icon" />
          <span>
            <kbd className="eth-keymap__kbd">{formatChord(c.chord)}</kbd> is also {c.with.map((o) => o.label).join(", ")}
          </span>
          {c.with.map((o) => (
            <Button key={o.id} size="sm" tone="ghost" onClick={() => onUnbindOther(o, c.chord)}>
              Remove from {o.label}
            </Button>
          ))}
        </div>
      ))}
    </div>
  );
}

/** "Press a shortcut…": records the next chord (Escape cancels). */
function Recorder({ action, onDone, keymapChords }: { action: KeymapAction; onDone(chord: string | null): void; keymapChords(chord: string): KeymapAction[] }) {
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
            <AlertTriangle aria-hidden className="eth-keymap__warn-icon" /> Also {clash.map((o) => o.label).join(", ")}
          </span>
        )}
        <Button size="sm" tone="accent" onClick={() => onDone(pending)}>
          {clash.length > 0 ? "Use anyway" : "Use"}
        </Button>
        <Button size="sm" tone="ghost" onClick={() => onDone(null)}>
          Cancel
        </Button>
      </span>
    );
  }
  return (
    <span className="eth-keymap__pending" data-testid="keymap-recording">
      <kbd className="eth-keymap__kbd eth-keymap__kbd--recording" aria-live="polite">
        Press a shortcut for {action.label}…
      </kbd>
      <Button size="sm" tone="ghost" onClick={() => onDone(null)}>
        Cancel
      </Button>
    </span>
  );
}
