// news: mounted once by the app shell. Checks for news a few seconds after launch (never
// blocking startup, never on the audio path) and hosts the viewer and the "News…" list.
// The viewer waits while a guided tour runs (#623; `NewsViewer`).
import { useEffect } from "react";
import { Toggle } from "@/kit";
import { useT } from "@/i18n";
import { NewsList, NewsViewer } from "./NewsDialog";
import { checkNews, setNewsEnabled, STARTUP_DELAY_MS, useNews, type FetchOptions } from "./store";
import "./news.css";

export function NewsRoot({ delayMs = STARTUP_DELAY_MS, fetchOptions }: { delayMs?: number; fetchOptions?: FetchOptions }) {
  useEffect(() => {
    const t = setTimeout(() => void checkNews(fetchOptions), delayMs);
    return () => clearTimeout(t);
  }, [delayMs, fetchOptions]);
  return (
    <>
      <NewsViewer />
      <NewsList />
    </>
  );
}

/** Settings › Privacy › News. */
export function NewsSettings() {
  const t = useT();
  const enabled = useNews((s) => s.enabled);
  return (
    <section className="eth-settings__section" aria-label={t("news.settings.title")} data-testid="settings-news">
      <h3 className="eth-settings__heading">{t("news.settings.title")}</h3>
      <p className="eth-settings__hint">{t("news.settings.hint")}</p>
      <Toggle size="sm" checked={enabled} onChange={setNewsEnabled} label={t("news.settings.check")} />
    </section>
  );
}
