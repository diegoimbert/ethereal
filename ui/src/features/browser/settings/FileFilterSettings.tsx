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
import { useCallback, useEffect, useState } from "react";
import { Plus, X } from "lucide-react";
import type { FileFilter, FileFilterCheck, FileFilterList } from "@/generated";
import { useEngineEvent } from "@/features/transport-bar/engine";
import { Button, IconButton, TextInput } from "@/kit";
import { cmd, commandErrorText, isCommandFailed, newId, type EngineTransport } from "@/transport";
import { useT, type MessageKey } from "@/i18n";

/** Live check delay after a keystroke. */
export const CHECK_DEBOUNCE_MS = 200;

interface Row {
  id: string;
  text: string;
}

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

  const isDefault = defaults !== null && sameFilter(draft.filter, defaults);
  const lib = current?.library;

  return (
    <section className="eth-settings__section eth-file-filter" aria-label={t("browser.filter.title")} data-testid="settings-library-filters">
      <div className="eth-settings__row">
        <h3 className="eth-settings__heading">{t("browser.filter.title")}</h3>
        <Button size="sm" tone="ghost" disabled={isDefault} onClick={() => void reset()}>
          {t("browser.filter.reset")}
        </Button>
      </div>
      <p className="eth-settings__hint">{t("browser.filter.hint")}</p>
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
                  <li key={row.id} className="eth-file-filter__rule">
                    <div className="eth-file-filter__rule-row">
                      <TextInput
                        size="sm"
                        className="eth-file-filter__input"
                        aria-label={t(rule, { n: i + 1 })}
                        aria-describedby={message ? errorId : undefined}
                        invalid={!!message}
                        spellCheck={false}
                        autoCapitalize="off"
                        autoComplete="off"
                        placeholder={t("browser.filter.placeholder")}
                        value={row.text}
                        autoFocus={row.text === "" && i === lists[list].length - 1}
                        onChange={(e) => edit(list, (rows) => rows.map((r) => (r.id === row.id ? { ...r, text: e.target.value } : r)))}
                        onBlur={() => {
                          const next = row.text.trim() === "" ? edit(list, (rows) => rows.filter((r) => r.id !== row.id)) : lists;
                          void save(next);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            e.preventDefault();
                            void save(lists);
                          }
                        }}
                      />
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
