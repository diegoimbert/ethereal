/**
 * Settings > General > Library: which files the library browser shows. Two
 * lists of regex rules on file names (`Browser::{Get,Set,Reset,Check}FileFilter`,
 * CONTRACTS.md §12.8): "Show" (whitelist; empty = every file) and "Hide" (blacklist, wins).
 *
 * - Edits are checked live (`CheckFileFilter`, debounced): a rule that doesn't compile is
 *   marked inline with the engine's message, and the status line says how many of the
 *   library's files the rules would show.
 * - A rule is saved when its field loses focus or on Enter, and on remove / reset; only
 *   when every rule compiles (blank rows are ignored and dropped on blur).
 * - "Reset to defaults" restores the engine's defaults (supported file types; system and
 *   sidecar files hidden). `FileFilterChanged` from elsewhere (another window) is followed.
 */
import "./fileFilterSettings.css";
import { useCallback, useEffect, useRef, useState, type FocusEvent } from "react";
import { Plus, X } from "lucide-react";
import type { FileFilter, FileFilterCheck, FileFilterList } from "@/generated";
import { useEngineEvent } from "@/features/transport-bar/engine";
import { Button, IconButton, Select, TextInput, Toggle, useConfirm } from "@/kit";
import { cmd, commandErrorText, isCommandFailed, newId, type EngineTransport } from "@/transport";
import { useT, type MessageKey } from "@/i18n";
import { fromRegex, RULE_KINDS, toRegex, type FriendlyRule, type RuleKind } from "./friendlyRules";

/** Live check delay after a keystroke. */
export const CHECK_DEBOUNCE_MS = 200;

interface Row {
  id: string;
  /** The rule (a regex: what the engine takes). */
  text: string;
  /** Edited in plain terms: the kind and text as typed (kept while typing, e.g. "wav, "). */
  friendly?: FriendlyRule;
}

/** Shows the rules as regexes (else in plain terms); kept on this device. */
const REGEX_MODE_KEY = "eth.library.regexRules";

function loadRegexMode(): boolean {
  try {
    return localStorage.getItem(REGEX_MODE_KEY) === "1";
  } catch {
    return false;
  }
}

const KIND_LABEL: Record<RuleKind | "regex", MessageKey> = {
  type: "browser.filter.kind.type",
  starts: "browser.filter.kind.starts",
  ends: "browser.filter.kind.ends",
  is: "browser.filter.kind.is",
  contains: "browser.filter.kind.contains",
  regex: "browser.filter.kind.regex",
};

type Lists = Record<FileFilterList, Row[]>;

const LISTS: ReadonlyArray<{
  list: FileFilterList;
  key: keyof FileFilter;
  title: MessageKey;
  hint: MessageKey;
  group: MessageKey;
  rule: MessageKey;
  remove: MessageKey;
  add: MessageKey;
}> = [
  {
    list: "Whitelist",
    key: "whitelist",
    title: "browser.filter.show",
    hint: "browser.filter.show_hint",
    group: "browser.filter.show_rules",
    rule: "browser.filter.show_rule",
    remove: "browser.filter.remove_show_rule",
    add: "browser.filter.add_show_rule",
  },
  {
    list: "Blacklist",
    key: "blacklist",
    title: "browser.filter.hide",
    hint: "browser.filter.hide_hint",
    group: "browser.filter.hide_rules",
    rule: "browser.filter.hide_rule",
    remove: "browser.filter.remove_hide_rule",
    add: "browser.filter.add_hide_rule",
  },
];

const toRows = (f: FileFilter): Lists => ({
  Whitelist: f.whitelist.map((text) => ({ id: newId(), text })),
  Blacklist: f.blacklist.map((text) => ({ id: newId(), text })),
});

/** The filter the rows describe (blank rows left out), and each kept row's id by list index. */
function toFilter(lists: Lists): { filter: FileFilter; ids: Record<FileFilterList, string[]> } {
  const kept = (rows: Row[]) => rows.filter((r) => r.text.trim() !== "");
  const w = kept(lists.Whitelist);
  const b = kept(lists.Blacklist);
  return {
    filter: { whitelist: w.map((r) => r.text), blacklist: b.map((r) => r.text) },
    ids: { Whitelist: w.map((r) => r.id), Blacklist: b.map((r) => r.id) },
  };
}

const sameFilter = (a: FileFilter, b: FileFilter) => JSON.stringify(a) === JSON.stringify(b);

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
  return e instanceof Error ? e.message : String(e);
}

export function FileFilterSettings({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const [saved, setSaved] = useState<FileFilter | null>(null);
  const [defaults, setDefaults] = useState<FileFilter | null>(null);
  const [lists, setLists] = useState<Lists | null>(null);
  const [checked, setChecked] = useState<{ key: string; check: FileFilterCheck } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [recount, setRecount] = useState(0);
  const [regexMode, setRegexModeState] = useState(loadRegexMode);
  /** The row a press is in (until that press's events are over): its field's blur keeps it. */
  const pressedRow = useRef<string | null>(null);
  const setRegexMode = (on: boolean) => {
    setRegexModeState(on);
    try {
      localStorage.setItem(REGEX_MODE_KEY, on ? "1" : "0");
    } catch {
      /* this session only */
    }
  };

  const adopt = useCallback((filter: FileFilter) => {
    setSaved(filter);
    setLists(toRows(filter));
  }, []);

  useEffect(() => {
    let live = true;
    transport
      .send(cmd("Browser", { type: "GetFileFilter" }))
      .then((r) => {
        if (!live || r.type !== "BrowserFileFilter") return;
        setDefaults(r.defaults);
        adopt(r.filter);
      })
      .catch((e: unknown) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [transport, adopt]);

  useEngineEvent((e) => {
    if (e.type !== "Browser") return;
    if (e.event.type === "FileFilterChanged") {
      const next = e.event.filter;
      // Another window changed it: follow, unless this one already shows it.
      if (!saved || !sameFilter(saved, next)) adopt(next);
    } else if (e.event.type === "IndexChanged") {
      setRecount((n) => n + 1);
    }
  });

  // Live check of the rows (validation + the library count).
  const draft = lists ? toFilter(lists) : null;
  const draftKey = draft ? JSON.stringify(draft.filter) : null;
  useEffect(() => {
    if (!draftKey) return;
    let live = true;
    const timer = setTimeout(() => {
      transport
        .send(cmd("Browser", { type: "CheckFileFilter", filter: JSON.parse(draftKey) as FileFilter, paths: [] }))
        .then((r) => {
          if (live && r.type === "BrowserFileFilterCheck") setChecked({ key: draftKey, check: r.check });
        })
        .catch(() => undefined);
    }, CHECK_DEBOUNCE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [transport, draftKey, recount]);

  if (!lists || !draft) return <p className="eth-settings__hint">{error ?? t("browser.filter.loading")}</p>;

  // Errors of the current rows only (a check of older text is ignored).
  const current = checked && checked.key === draftKey ? checked.check : null;
  const rowError = new Map<string, string>();
  for (const e of current?.errors ?? []) {
    const id = draft.ids[e.list][e.index];
    if (id) rowError.set(id, e.message);
  }

  const save = async (next: Lists) => {
    const { filter } = toFilter(next);
    if (saved && sameFilter(filter, saved)) return;
    try {
      const r = await transport.send(cmd("Browser", { type: "CheckFileFilter", filter, paths: [] }));
      if (r.type === "BrowserFileFilterCheck" && r.check.errors.length > 0) return; // shown inline
      const reply = await transport.send(cmd("Browser", { type: "SetFileFilter", filter }));
      if (reply.type === "BrowserFileFilter") setSaved(reply.filter);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const askReset = () =>
    useConfirm.getState().ask({
      title: t("browser.filter.reset_confirm.title"),
      body: t("browser.filter.reset_confirm.body"),
      action: t("browser.filter.reset_confirm.action"),
      danger: true,
      run: () => void reset(),
    });

  const reset = async () => {
    try {
      const reply = await transport.send(cmd("Browser", { type: "ResetFileFilter" }));
      if (reply.type === "BrowserFileFilter") {
        setDefaults(reply.defaults);
        adopt(reply.filter);
      }
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const edit = (list: FileFilterList, update: (rows: Row[]) => Row[]) => {
    const next = { ...lists, [list]: update(lists[list]) };
    setLists(next);
    return next;
  };

  /**
   * A rule's field lost focus: save, dropping it when blank, unless the focus stays in its row
   * or the blur comes from a press in it (its kind menu: WebKit doesn't focus a pressed button,
   * so the press is what tells; it comes before the blur in every browser).
   */
  const leaveRow = (e: FocusEvent<HTMLElement>, list: FileFilterList, row: Row) => {
    const to = e.relatedTarget;
    if (to instanceof Node && e.currentTarget.closest(".eth-file-filter__rule")?.contains(to)) return;
    if (pressedRow.current === row.id) return;
    const current = lists[list].find((r) => r.id === row.id) ?? row;
    const next = current.text.trim() === "" ? edit(list, (rows) => rows.filter((r) => r.id !== row.id)) : lists;
    void save(next);
  };

  const isDefault = defaults !== null && sameFilter(draft.filter, defaults);
  const lib = current?.library;

  return (
    <section className="eth-settings__section eth-file-filter" aria-label={t("browser.filter.title")} data-testid="settings-library-filters">
      <div className="eth-settings__row">
        <h3 className="eth-settings__heading">{t("browser.filter.title")}</h3>
        <Button size="sm" disabled={isDefault} onClick={askReset} data-testid="file-filter-reset">
          {t("browser.filter.reset")}
        </Button>
      </div>
      <p className="eth-settings__hint">{t("browser.filter.hint")}</p>
      <Toggle size="sm" label={t("browser.filter.use_regex")} checked={regexMode} onChange={setRegexMode} />
      {regexMode && <p className="eth-settings__hint">{t("browser.filter.regex_hint")}</p>}
      {error && (
        <p className="eth-file-filter__error" role="alert">
          {error}
        </p>
      )}
      <div className="eth-file-filter__lists">
        {LISTS.map(({ list, title, hint, group, rule, remove, add }) => (
          <div key={list} className="eth-file-filter__list" role="group" aria-label={t(group)}>
            <div className="eth-file-filter__list-head">
              <span className="eth-file-filter__title">{t(title)}</span>
              <span className="eth-file-filter__list-hint">{t(hint)}</span>
            </div>
            <ul className="eth-file-filter__rules">
              {lists[list].map((row, i) => {
                const message = rowError.get(row.id);
                const errorId = `eth-file-filter-${row.id}`;
                return (
                  <li
                    key={row.id}
                    className="eth-file-filter__rule"
                    onPointerDownCapture={() => {
                      pressedRow.current = row.id;
                      setTimeout(() => {
                        if (pressedRow.current === row.id) pressedRow.current = null;
                      }, 0);
                    }}
                  >
                    <div className="eth-file-filter__rule-row">
                      {regexMode ? (
                        <TextInput
                          size="sm"
                          className="eth-file-filter__input eth-file-filter__input--regex"
                          aria-label={t(rule, { n: i + 1 })}
                          aria-describedby={message ? errorId : undefined}
                          invalid={!!message}
                          spellCheck={false}
                          autoCapitalize="off"
                          autoComplete="off"
                          placeholder={t("browser.filter.placeholder")}
                          value={row.text}
                          autoFocus={row.text === "" && i === lists[list].length - 1}
                          onChange={(e) => edit(list, (rows) => rows.map((r) => (r.id === row.id ? { id: r.id, text: e.target.value } : r)))}
                          onBlur={(e) => leaveRow(e, list, row)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.preventDefault();
                              void save(lists);
                            }
                          }}
                        />
                      ) : (
                        <FriendlyRuleEditor
                          row={row}
                          label={t(rule, { n: i + 1 })}
                          invalid={message ? errorId : undefined}
                          autoFocus={row.text === "" && !row.friendly?.value && i === lists[list].length - 1}
                          onChange={(next, commit) => {
                            const rows = edit(list, (rs) => rs.map((r) => (r.id === row.id ? next : r)));
                            if (commit) void save(rows);
                          }}
                          onBlur={(e) => leaveRow(e, list, row)}
                          onEnter={() => void save(lists)}
                        />
                      )}
                      <IconButton
                        size="sm"
                        tone="ghost"
                        label={t(remove, { n: i + 1 })}
                        icon={<X />}
                        onClick={() => void save(edit(list, (rows) => rows.filter((r) => r.id !== row.id)))}
                      />
                    </div>
                    {message && (
                      <span id={errorId} className="eth-file-filter__rule-error">
                        {message}
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>
            <Button size="sm" tone="ghost" className="eth-file-filter__add" onClick={() => edit(list, (rows) => [...rows, { id: newId(), text: "" }])}>
              <Plus aria-hidden />
              {t(add)}
            </Button>
          </div>
        ))}
      </div>
      <p className="eth-settings__hint" role="status" data-testid="file-filter-count">
        {current && current.errors.length > 0
          ? t("browser.filter.fix_rules")
          : lib
            ? lib.total === 0
              ? t("browser.filter.no_files")
              : t("browser.filter.count", { shown: lib.shown, total: lib.total, hidden: lib.total - lib.shown })
            : "\u00a0"}
      </p>
    </section>
  );
}

/**
 * A rule in plain terms: its kind (a file type, a name that starts with / ends with / is /
 * contains) and its text. A rule that is none of those (a custom pattern) shows its regex.
 */
function FriendlyRuleEditor({
  row,
  label,
  invalid,
  autoFocus,
  onChange,
  onBlur,
  onEnter,
}: {
  row: Row;
  label: string;
  /** The id of its error message, when it has one. */
  invalid: string | undefined;
  autoFocus: boolean;
  /** The edited row; `commit`: save now (a kind picked). */
  onChange(row: Row, commit: boolean): void;
  onBlur(e: FocusEvent<HTMLElement>): void;
  onEnter(): void;
}) {
  const t = useT();
  const shown: FriendlyRule | null = row.friendly ?? (row.text === "" ? { kind: "type", value: "" } : fromRegex(row.text));
  const kind: RuleKind | "regex" = shown?.kind ?? "regex";
  const value = shown ? shown.value : row.text;
  const setKind = (k: RuleKind | "regex") => {
    if (k === "regex") return onChange({ id: row.id, text: row.text }, true);
    const friendly = { kind: k, value: shown?.value ?? "" };
    onChange({ id: row.id, text: toRegex(friendly), friendly }, true);
  };
  const setValue = (v: string) => {
    if (kind === "regex") return onChange({ id: row.id, text: v }, false);
    const friendly = { kind, value: v };
    onChange({ id: row.id, text: toRegex(friendly), friendly }, false);
  };
  return (
    <>
      <Select<RuleKind | "regex">
        size="sm"
        className="eth-file-filter__kind"
        aria-label={t("browser.filter.kind_of", { rule: label })}
        value={kind}
        options={[...RULE_KINDS, ...(kind === "regex" ? (["regex"] as const) : [])].map((k) => ({ value: k, label: t(KIND_LABEL[k]) }))}
        onChange={setKind}
      />
      <TextInput
        size="sm"
        className={kind === "regex" ? "eth-file-filter__input eth-file-filter__input--regex" : "eth-file-filter__input"}
        aria-label={label}
        aria-describedby={invalid}
        invalid={!!invalid}
        spellCheck={false}
        autoCapitalize="off"
        autoComplete="off"
        placeholder={
          kind === "type" ? t("browser.filter.placeholder_type") : kind === "regex" ? t("browser.filter.placeholder") : t("browser.filter.placeholder_text")
        }
        value={value}
        autoFocus={autoFocus}
        onChange={(e) => setValue(e.target.value)}
        onBlur={onBlur}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onEnter();
          }
        }}
      />
    </>
  );
}
