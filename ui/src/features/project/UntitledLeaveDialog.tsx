// "Keep or discard this untitled project?" (`untitledLeave.ts`): asked when leaving an
// untitled project (`UntitledLeaveDialog`, mounted by the project menu) and, on the desktop,
// in place of the quit confirmation (`features/quit`, with `UntitledPrompt`).
import { useState, type FormEvent } from "react";
import { Trans, useT } from "@/i18n";
import { Button, Dialog, TextInput } from "@/kit";
import { answerUntitled, useUntitledLeave, type UntitledChoice, type UntitledLeaveKind } from "./untitledLeave";

/** The question for project `name` (its current, default name). */
export function UntitledPrompt({
  open,
  name,
  kind,
  onChoice,
}: {
  open: boolean;
  name: string;
  kind: UntitledLeaveKind;
  onChoice(choice: UntitledChoice): void;
}) {
  const t = useT();
  const [typed, setTyped] = useState("");
  const choose = (choice: UntitledChoice) => {
    setTyped("");
    onChoice(choice);
  };
  const keep = (e?: FormEvent) => {
    e?.preventDefault();
    choose({ type: "keep", name: typed });
  };
  const quit = kind === "quit";
  return (
    <Dialog
      open={open}
      onClose={() => choose({ type: "cancel" })}
      title={t("project.untitled.title")}
      testId="untitled-leave"
      footer={
        <>
          <Button onClick={() => choose({ type: "cancel" })}>{t("project.actions.cancel")}</Button>
          <Button tone="danger" onClick={() => choose({ type: "discard" })}>
            {t(quit ? "project.untitled.discard_quit" : "project.untitled.discard")}
          </Button>
          <Button tone="accent" onClick={() => keep()}>
            {t(quit ? "project.untitled.keep_quit" : "project.untitled.keep")}
          </Button>
        </>
      }
    >
      <form className="eth-untitled" onSubmit={keep}>
        <p className="eth-project-screen__hint">
          <Trans k="project.untitled.body" values={{ name: <span className="eth-user-text">{name}</span> }} />
        </p>
        <TextInput aria-label={t("project.untitled.name")} placeholder={name} value={typed} autoFocus onChange={(e) => setTyped(e.target.value)} />
      </form>
    </Dialog>
  );
}

/** The question while leaving a project (mounted once, by the project menu). */
export function UntitledLeaveDialog() {
  const pending = useUntitledLeave((s) => s.pending);
  return <UntitledPrompt open={pending !== null} name={pending?.name ?? ""} kind={pending?.kind ?? "leave"} onChoice={answerUntitled} />;
}
