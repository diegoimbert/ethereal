import { AudioLines, BookOpen, CircleCheck, Music } from "lucide-react";
import { useContributions, type GetStartedContribution } from "@/ext";
import { useState, type ReactNode } from "react";
import { Button } from "@/kit";
import type { EngineCommands } from "@/features/transport-bar/engine";
import { platformOf, RECORD_AUDIO_TOUR, useTour, useTutorialProgress, type TourDef } from "@/features/tutorials";
import { openDocs } from "@/features/docs/open";
import { useStartTour } from "./useStartTour";
import { useT, type MessageKey } from "@/i18n";

/** The welcome screen's guides: guided tours of the real UI (docs/TUTORIALS.md), or coming soon. */
const ARTICLES: ReadonlyArray<Article> = [
  // Waits for the default sample pack (#519).
  { title: "project.start.first_track", blurb: "project.start.first_track_blurb", icon: <Music aria-hidden /> },
  { title: "tutorials.record.title", blurb: "project.start.record_blurb", icon: <AudioLines aria-hidden />, tour: RECORD_AUDIO_TOUR },
];

interface Article {
  title: MessageKey | (() => string);
  blurb: MessageKey | (() => string);
  icon: ReactNode;
  tour?: TourDef;
}

/** The app's guides with the extensions' (`@/ext` "project.getStarted") placed in. */
function articles(extra: ReadonlyArray<GetStartedContribution>): Article[] {
  const out: Article[] = [...ARTICLES];
  for (const x of extra) {
    const at = x.after ? out.findIndex((a) => a.tour?.id === x.after) : -1;
    out.splice(at < 0 ? out.length : at + 1, 0, { title: x.title, blurb: x.blurb, icon: x.icon, tour: x.tour });
  }
  return out;
}

/**
 * The welcome screen's "Get started": each guide is a guided tour. Starting one makes a
 * project for it, hides the welcome screen and walks through the app; finishing or leaving
 * it comes back here. A finished one shows a check mark and asks before running again.
 */
export function GetStarted({ commands, onStarted }: { commands: EngineCommands; onStarted(): void }) {
  const t = useT();
  const done = useTutorialProgress((s) => s.done);
  const extra = useContributions("project.getStarted");
  const hosts = useTour((s) => s.hosts);
  const [confirm, setConfirm] = useState<string | null>(null);
  const transport = commands.transport;
  const tours = useStartTour(commands, onStarted);
  const { busy, ready } = tours;
  const start = (tour: TourDef) => {
    setConfirm(null);
    tours.start(tour);
  };

  return (
    <section className="eth-welcome__start" aria-label={t("project.start.title")}>
      <span className="eth-project-screen__label">{t("project.start.title")}</span>
      <ul className="eth-welcome__articles">
        {articles(extra).map((a) => {
          const tour = a.tour;
          const title = typeof a.title === "function" ? a.title() : t(a.title);
          const finished = tour !== undefined && `${tour.id}@${tour.version}` in done;
          const available = tour !== undefined && hosts > 0 && !!transport && tour.platforms.includes(platformOf(transport.kind));
          const body = (
            <>
              <span className="eth-welcome__article-icon">{a.icon}</span>
              <span className="eth-welcome__article-text">
                <span className="eth-welcome__article-title">{title}</span>
                <span className="eth-welcome__article-blurb">{typeof a.blurb === "function" ? a.blurb() : t(a.blurb)}</span>
              </span>
            </>
          );
          if (!tour || !available)
            return (
              <li key={tour?.id ?? title} className="eth-welcome__article" data-soon>
                {body}
                <span className="eth-welcome__soon">{t("project.start.soon")}</span>
              </li>
            );
          if (confirm === tour.id)
            return (
              <li key={tour?.id ?? title} className="eth-welcome__article eth-welcome__article--confirm" data-testid={`tutorial-confirm-${tour.id}`}>
                <span className="eth-welcome__article-icon">{a.icon}</span>
                <span className="eth-welcome__article-text" role="alert">
                  <span className="eth-welcome__article-title">{title}</span>
                  <span className="eth-welcome__article-blurb">{t("project.start.again_question")}</span>
                </span>
                <Button size="sm" onClick={() => setConfirm(null)}>
                  {t("project.actions.cancel")}
                </Button>
                <Button size="sm" tone="accent" disabled={busy} onClick={() => start(tour)} autoFocus>
                  {t("project.start.again")}
                </Button>
              </li>
            );
          return (
            <li key={tour?.id ?? title} className="eth-welcome__article-item">
              <button
                type="button"
                className="eth-welcome__article eth-welcome__article--tour"
                data-done={finished || undefined}
                disabled={busy || !ready}
                aria-label={finished ? t("project.start.completed_named", { title }) : title}
                onClick={() => (finished ? setConfirm(tour.id) : start(tour))}
              >
                {body}
                {finished && (
                  <span className="eth-welcome__done" title={t("project.start.completed")}>
                    <CircleCheck aria-hidden />
                  </span>
                )}
              </button>
            </li>
          );
        })}
        {/* docs (#665): the user documentation, every feature explained. */}
        <li className="eth-welcome__article-item">
          <button type="button" className="eth-welcome__article eth-welcome__article--tour" data-testid="welcome-docs" onClick={() => openDocs()}>
            <span className="eth-welcome__article-icon">
              <BookOpen aria-hidden />
            </span>
            <span className="eth-welcome__article-text">
              <span className="eth-welcome__article-title">{t("project.start.docs")}</span>
              <span className="eth-welcome__article-blurb">{t("project.start.docs_blurb")}</span>
            </span>
          </button>
        </li>
      </ul>
    </section>
  );
}
