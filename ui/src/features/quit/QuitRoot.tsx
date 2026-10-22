// Desktop: asks "Quit Ethereal?" when the shell reports a quit request (⌘Q, the main
// window's close button), and closes the topmost floating window on ⌘W in the main window.
// With an untitled project open (`features/project/untitledLeave.ts`) the question is "keep
// or discard it?" instead: discarding has the engine delete it as it stops.
// Mounted once by the app shell; renders nothing outside the desktop shell.
import { useEffect } from "react";
import { Trans, useT } from "@/i18n";
import { Button, closeTopFloatingWindow, Dialog } from "@/kit";
import { draftsTransport } from "@/features/project/drafts";
import { discardUntitled, keepUntitled, untitledOpen, type UntitledChoice } from "@/features/project/untitledLeave";
import { UntitledPrompt } from "@/features/project/UntitledLeaveDialog";
import { useProjectStore } from "@/state";
import { quitBridge, type QuitBridge } from "./bridge";
import { useQuitConfirm } from "./store";
import "./quit.css";

export function QuitRoot() {
  const bridge = quitBridge();
  return bridge ? <QuitConfirm bridge={bridge} /> : null;
}

function QuitConfirm({ bridge }: { bridge: QuitBridge }) {
  const t = useT();
  const open = useQuitConfirm((s) => s.open);
  const setOpen = useQuitConfirm((s) => s.setOpen);
  const dirty = useProjectStore((s) => s.dirty);
  const name = useProjectStore((s) => s.project?.settings.name);
  // Read as the question opens.
  const untitled = open ? untitledOpen() : null;

  useEffect(() => {
    let alive = true;
    const offs: Array<() => void> = [];
    const keep = (off: () => void) => {
      if (alive) offs.push(off);
      else off();
    };
    void bridge.onQuitRequested(() => useQuitConfirm.getState().setOpen(true)).then(keep);
    void bridge
      .onCloseRequested(() => {
        // A dialog showing (this one included) is left alone.
        if (!useQuitConfirm.getState().open) closeTopFloatingWindow();
      })
      .then(keep);
    // Never turned off on unmount: invokes may reach the shell out of order (StrictMode's
    // remount would race), and a request nobody answers quits on the next ⌘Q after a while.
    void bridge.setGuard(true);
    return () => {
      alive = false;
      offs.forEach((off) => off());
    };
  }, [bridge]);

  const cancel = () => {
    setOpen(false);
    void bridge.cancel();
  };
  const quit = () => {
    setOpen(false);
    void bridge.quit();
  };

  /** The untitled project's answer: kept (maybe named) or discarded, then quit. */
  const answer = async (choice: UntitledChoice) => {
    const t = draftsTransport();
    if (choice.type === "cancel" || !untitled || !t) return cancel();
    setOpen(false);
    if (choice.type === "keep") await keepUntitled(t, untitled, choice.name).catch(() => undefined);
    else await discardUntitled(t, untitled.id).catch(() => undefined);
    void bridge.quit();
  };

  // Enter quits (unless a button has the focus: it does its own job). The untitled
  // question's name field submits its own form (keep).
  const asksUntitled = untitled !== null;
  useEffect(() => {
    if (!open || asksUntitled) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Enter" || e.defaultPrevented || (e.target instanceof Element && e.target.closest("button"))) return;
      e.preventDefault();
      setOpen(false);
      void bridge.quit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, asksUntitled, bridge, setOpen]);

  if (untitled) return <UntitledPrompt open name={untitled.settings.name} kind="quit" onChoice={(c) => void answer(c)} />;
  return (
    <Dialog
      open={open}
      onClose={cancel}
      title={t("shell.quit.title")}
      className="eth-quit"
      footer={
        <>
          <Button onClick={cancel}>{t("shell.quit.cancel")}</Button>
          <Button tone="accent" onClick={quit}>
            {t("shell.quit.quit")}
          </Button>
        </>
      }
    >
      {/* Never an empty body: what quitting does to the project, saved or not. */}
      <p className="eth-quit__text">
        {name ? (
          <Trans k={dirty ? "shell.quit.dirty_named" : "shell.quit.clean_named"} values={{ name: <span className="eth-user-text">{name}</span> }} />
        ) : (
          t(dirty ? "shell.quit.dirty" : "shell.quit.clean")
        )}
      </p>
    </Dialog>
  );
}
