import { useEffect } from "react";
import { createPortal } from "react-dom";
import { formatChord, formatKeys } from "./chords";
import { useT } from "@/i18n";
import { actionLabel, groupActions, groupLabel, scopesLabel, useActions } from "./grouping";
import { MOUSE_GESTURES } from "./groups";
import { PRINTING_CLASS, usePrinting } from "./print";
import { effectiveChords, PRESET_LABELS, useKeymapStore } from "./store";

/** The cheat sheet's content (also used by tests and the print view). */
export function CheatSheetContent() {
  const t = useT();
  const keymap = useKeymapStore((s) => s.keymap);
  const actions = useActions().filter((a) => effectiveChords(a, keymap).length > 0);
  return (
    <div className="eth-keymap-sheet" data-testid="keymap-cheat-sheet">
      <header className="eth-keymap-sheet__header">
        <h1 className="eth-keymap-sheet__title">{t("shell.keymap.sheet_title")}</h1>
        <span className="eth-keymap-sheet__preset">
          {t(keymap.overrides.length > 0 ? "shell.keymap.sheet_preset_customized" : "shell.keymap.sheet_preset", { preset: t(PRESET_LABELS[keymap.preset]) })}
        </span>
      </header>
      <div className="eth-keymap-sheet__columns">
        {groupActions(actions).map(({ group, actions: rows }) => (
          <section key={group} className="eth-keymap-sheet__group">
            <h2 className="eth-keymap-sheet__heading">{groupLabel(group)}</h2>
            <dl className="eth-keymap-sheet__list">
              {rows.map((a) => (
                <div key={a.id} className="eth-keymap-sheet__row">
                  <dt>
                    {actionLabel(a)}
                    {scopesLabel(a) && <span className="eth-keymap-sheet__scope"> ({scopesLabel(a)})</span>}
                  </dt>
                  <dd>
                    {effectiveChords(a, keymap).map((c) => (
                      <kbd key={c} className="eth-keymap-sheet__kbd">
                        {formatChord(c)}
                      </kbd>
                    ))}
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
        <section className="eth-keymap-sheet__group" data-testid="keymap-mouse-gestures">
          <h2 className="eth-keymap-sheet__heading">{t("shell.keymap.sheet_mouse")}</h2>
          <dl className="eth-keymap-sheet__list">
            {MOUSE_GESTURES.map(({ label, keys }) => (
              <div key={label} className="eth-keymap-sheet__row">
                <dt>{t(label)}</dt>
                <dd>
                  <kbd className="eth-keymap-sheet__kbd">{t("shell.keymap.sheet_drag", { keys: formatKeys(keys) })}</kbd>
                </dd>
              </div>
            ))}
          </dl>
        </section>
      </div>
    </div>
  );
}

/** Mounted once (by `KeymapRoot`): renders the sheet and prints it when asked. */
export function CheatSheetPrinter() {
  const printing = usePrinting((s) => s.printing);
  useEffect(() => {
    if (!printing) return;
    document.body.classList.add(PRINTING_CLASS);
    const done = () => {
      document.body.classList.remove(PRINTING_CLASS);
      usePrinting.setState({ printing: false });
    };
    window.addEventListener("afterprint", done, { once: true });
    // Print once the sheet is in the page. `print()` blocks in most browsers; where it
    // doesn't (or is missing), `afterprint` cleans up.
    const t = setTimeout(() => {
      if (typeof window.print === "function") window.print();
      else done();
    }, 0);
    return () => {
      clearTimeout(t);
      window.removeEventListener("afterprint", done);
      document.body.classList.remove(PRINTING_CLASS);
    };
  }, [printing]);
  if (!printing) return null;
  return createPortal(
    <div className="eth-keymap-print" data-theme="light">
      <CheatSheetContent />
    </div>,
    document.body,
  );
}
