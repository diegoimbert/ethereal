// The archived projects and the bin, under the recent projects in the project picker
// (`Project::{SetArchived, MoveToBin, RestoreFromBin, ListBin, EmptyBin}`): each a folded
// section with its count. Archived projects open, come back to the recents, or go to the bin;
// projects in the bin are restored or deleted for good (with the extensions' delete choices),
// and the engine deletes them after 30 days.
import { useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { ProjectSummary } from "@/generated";
import { errorMessage, type EngineCommands } from "@/features/transport-bar/engine";
import { Button } from "@/kit";
import { Trans, useT } from "@/i18n";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { moveToBin } from "./actions";
import { DeleteChoices, DeleteChoiceSwitches } from "./deleteChoices";
import { daysLeft, formatModified, sortProjects } from "./projectNames";

/** The projects in the bin, re-read whenever the project list changes. */
function useBin(transport: EngineTransport | null): { projects: ProjectSummary[]; now: number } {
  const projects = useProjectStore((s) => s.projects);
  const [bin, setBin] = useState<{ projects: ProjectSummary[]; now: number }>({ projects: [], now: 0 });
  useEffect(() => {
    if (!transport) return;
    let active = true;
    transport.send(cmd("Project", { type: "ListBin" })).then(
      // When it was read: the days left are counted from then.
      (r) => active && r.type === "Projects" && setBin({ projects: r.projects, now: Date.now() }),
      () => undefined,
    );
    return () => {
      active = false;
    };
  }, [transport, projects]);
  return bin;
}

export function ProjectShelves({
  commands,
  openProject,
  setError,
}: {
  commands: EngineCommands;
  openProject(id: string): void;
  setError(message: string | null): void;
}) {
  const t = useT();
  const { send, transport } = commands;
  const projects = useProjectStore((s) => s.projects);
  const current = useProjectStore((s) => s.project);
  const archived = useMemo(() => sortProjects(projects).filter((p) => p.archived && p.id !== current?.id), [projects, current?.id]);
  const { projects: bin, now } = useBin(transport);
  const [shown, setShown] = useState<"archived" | "bin" | null>(null);
  // The row asking to delete for good (with the extensions' choices), or the whole bin.
  const [confirm, setConfirm] = useState<{ id: string; choices: DeleteChoices } | "all" | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async (command: Parameters<typeof send>[0]) => {
    setBusy(true);
    const reply = await send(command);
    setBusy(false);
    return reply;
  };
  const toBin = async (p: ProjectSummary) => {
    if (!transport) return;
    setBusy(true);
    try {
      await moveToBin(transport, p);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  if (archived.length === 0 && bin.length === 0) return null;
  const toggle = (which: "archived" | "bin") => setShown((s) => (s === which ? null : which));
  const head = (which: "archived" | "bin", label: string) => (
    <button type="button" className="eth-project-shelves__head" aria-expanded={shown === which} onClick={() => toggle(which)}>
      {shown === which ? <ChevronDown aria-hidden /> : <ChevronRight aria-hidden />}
      <span className="eth-project-screen__label">{label}</span>
    </button>
  );

  return (
    <div className="eth-project-shelves">
      {archived.length > 0 && (
        <section aria-label={t("project.archive.title")}>
          {head("archived", t("project.archive.count", { count: archived.length }))}
          {shown === "archived" && (
            <ul className="eth-project-screen__list">
              {archived.map((p) => (
                <li key={p.id} className="eth-project-screen__item eth-project-shelves__item">
                  <button
                    type="button"
                    className="eth-project-screen__open"
                    aria-label={t("project.actions.open_named", { name: p.name })}
                    disabled={busy}
                    onClick={() => openProject(p.id)}
                  >
                    <span className="eth-project-screen__name eth-user-text" title={p.name}>
                      {p.name}
                    </span>
                    {/* A date in the user's language (Intl), not catalog text. */}
                    <span className="eth-project-screen__date" data-i18n-exempt>
                      {formatModified(p.modified_ms)}
                    </span>
                  </button>
                  <Button size="sm" tone="ghost" disabled={busy} onClick={() => void run(cmd("Project", { type: "SetArchived", id: p.id, archived: false }))}>
                    {t("project.archive.unarchive")}
                  </Button>
                  <Button size="sm" tone="ghost" disabled={busy} aria-label={t("project.bin.move_named", { name: p.name })} onClick={() => void toBin(p)}>
                    {t("project.bin.move")}
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </section>
      )}
      {bin.length > 0 && (
        <section aria-label={t("project.bin.title")}>
          {head("bin", t("project.bin.count", { count: bin.length }))}
          {shown === "bin" && (
            <>
              <p className="eth-project-shelves__note">
                {t("project.bin.note")}
                {confirm === "all" ? (
                  <span className="eth-project-screen__confirm">
                    <span>{t("project.bin.empty_confirm", { count: bin.length })}</span>
                    <Button size="sm" onClick={() => setConfirm(null)}>
                      {t("project.actions.cancel")}
                    </Button>
                    <Button
                      size="sm"
                      className="eth-project-screen__danger"
                      disabled={busy}
                      onClick={() => {
                        setConfirm(null);
                        void run(cmd("Project", { type: "EmptyBin" }));
                      }}
                    >
                      {t("project.bin.empty")}
                    </Button>
                  </span>
                ) : (
                  <Button size="sm" tone="ghost" disabled={busy} onClick={() => setConfirm("all")}>
                    {t("project.bin.empty_ask")}
                  </Button>
                )}
              </p>
              <ul className="eth-project-screen__list">
                {bin.map((p) => (
                  <li key={p.id} className="eth-project-screen__item eth-project-shelves__item">
                    {confirm !== "all" && confirm?.id === p.id ? (
                      <span className="eth-project-screen__confirm">
                        <span className="eth-project-screen__name">
                          <Trans k="project.actions.delete_named" values={{ name: <span className="eth-user-text">{p.name}</span> }} />
                        </span>
                        <DeleteChoiceSwitches choices={confirm.choices} />
                        <Button size="sm" onClick={() => setConfirm(null)}>
                          {t("project.actions.cancel")}
                        </Button>
                        <Button
                          size="sm"
                          className="eth-project-screen__danger"
                          disabled={busy}
                          aria-label={t("project.actions.confirm_delete", { name: p.name })}
                          onClick={() => {
                            const { choices } = confirm;
                            setConfirm(null);
                            void run(cmd("Project", { type: "Delete", id: p.id })).then((r) => r && choices.runChecked((e) => setError(errorMessage(e))));
                          }}
                        >
                          {t("project.actions.delete")}
                        </Button>
                      </span>
                    ) : (
                      <>
                        <span className="eth-project-screen__name eth-user-text" title={p.name}>
                          {p.name}
                        </span>
                        <span className="eth-project-screen__date">{t("project.bin.days_left", { count: daysLeft(p.binned_ms ?? now, now) })}</span>
                        <Button
                          size="sm"
                          tone="ghost"
                          disabled={busy}
                          aria-label={t("project.bin.restore_named", { name: p.name })}
                          onClick={() => void run(cmd("Project", { type: "RestoreFromBin", id: p.id }))}
                        >
                          {t("project.bin.restore")}
                        </Button>
                        <Button
                          size="sm"
                          tone="ghost"
                          disabled={busy}
                          aria-label={t("project.bin.delete_named", { name: p.name })}
                          onClick={() => setConfirm({ id: p.id, choices: new DeleteChoices(p) })}
                        >
                          {t("project.bin.delete")}
                        </Button>
                      </>
                    )}
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>
      )}
    </div>
  );
}
