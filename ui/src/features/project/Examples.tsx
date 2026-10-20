import { AudioWaveform, CircleCheck, Disc3, Drum, Piano } from "lucide-react";
import { useState, type ReactNode } from "react";
import { Button } from "@/kit";
import { notify } from "@/features/notifications";
import { errorMessage, type EngineCommands } from "@/features/transport-bar/engine";
import { EXAMPLES, platformOf, useTour, useTutorialProgress, type ExampleDef } from "@/features/tutorials";
import { useT } from "@/i18n";
import { useProjectStore } from "@/state";
import { cmd, newProjectId } from "@/transport";
import { guardLeave } from "./leaveGuard";
import { uniqueName } from "./projectNames";
import { useStartTour } from "./useStartTour";

const ICONS: Readonly<Record<string, ReactNode>> = {
  "house-beat": <Drum aria-hidden />,
  "hip-hop-loop": <Disc3 aria-hidden />,
  "lofi-chords": <Piano aria-hidden />,
  "simple-bassline": <AudioWaveform aria-hidden />,
};

/**
 * The welcome screen's example beats (docs/TUTORIALS.md "Example beats"): short loops
 * made of the built-in instruments, as factory project templates. A click opens one as a new
 * project; "show me how" starts its guided tour (a project of its own, walked through part by
 * part, ending on the docs guide that rebuilds it from an empty project).
 */
export function Examples({ commands, onStarted }: { commands: EngineCommands; onStarted(): void }) {
  const t = useT();
  const done = useTutorialProgress((s) => s.done);
  const hosts = useTour((s) => s.hosts);
  const tours = useStartTour(commands, onStarted);
  const [opening, setOpening] = useState(false);
  const transport = commands.transport;
  const busy = opening || tours.busy || !tours.ready;

  const open = (ex: ExampleDef) => {
    if (!transport) return;
    void guardLeave(async () => {
      setOpening(true);
      try {
        const name = uniqueName(t(ex.title), useProjectStore.getState().projects);
        await transport.send(cmd("Template", { type: "NewProject", id: newProjectId(), name, template: ex.template }));
        onStarted();
      } catch (e) {
        notify("Error", t("project.examples.failed", { error: errorMessage(e) }));
      } finally {
        setOpening(false);
      }
    }, t("project.leave.create"));
  };

  return (
    <section className="eth-welcome__start" aria-label={t("project.examples.title")} data-testid="welcome-examples">
      <span className="eth-project-screen__label">{t("project.examples.title")}</span>
      <ul className="eth-welcome__articles">
        {EXAMPLES.map((ex) => {
          const title = t(ex.title);
          const tour = ex.tour;
          const finished = `${tour.id}@${tour.version}` in done;
          const guided = hosts > 0 && !!transport && tour.platforms.includes(platformOf(transport.kind));
          return (
            <li key={ex.slug} className="eth-welcome__article-item eth-welcome__example" data-example={ex.slug}>
              <button
                type="button"
                className="eth-welcome__article eth-welcome__article--tour"
                disabled={busy || !transport}
                aria-label={t("project.examples.open_named", { title })}
                onClick={() => open(ex)}
              >
                <span className="eth-welcome__article-icon">{ICONS[ex.slug]}</span>
                <span className="eth-welcome__article-text">
                  <span className="eth-welcome__article-title">{title}</span>
                  <span className="eth-welcome__article-blurb">{t(ex.blurb)}</span>
                </span>
              </button>
              {guided && (
                <Button
                  size="sm"
                  tone="ghost"
                  className="eth-welcome__example-guide"
                  data-done={finished || undefined}
                  disabled={busy}
                  aria-label={finished ? t("project.examples.guide_done_named", { title }) : t("project.examples.guide_named", { title })}
                  title={t("project.examples.guide_title")}
                  onClick={() => tours.start(tour)}
                >
                  {finished && <CircleCheck aria-hidden />}
                  {t("project.examples.guide")}
                </Button>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
