// news: the viewer (one item at a time, in a sandboxed iframe) and the "News…" list.
//
// Security model (docs/NEWS.md): the item page is loaded from the news service's origin in an
// iframe sandboxed WITHOUT `allow-same-origin`, so it runs in an opaque origin: it can't read
// the app's DOM, storage or cookies, nor reach the desktop shell's IPC (Tauri only answers the
// app's own origin; no capability lists a remote URL). The service also sends a CSP with the
// same `sandbox` directive. Links leave through popups (a new tab on web, the system browser
// on desktop). The page may post `{type: "ethereal-news", action: "close"}` or
// `{…, action: "resize", height}`; nothing else is read.
import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { Button, Dialog, IconButton } from "@/kit";
import { formatDate, useT } from "@/i18n";
import { closeNews, closeNewsList, nextNews, showNewsItem, useNews, type NewsItem } from "./store";
import { useTour } from "@/features/tutorials/store";
import { openSettings } from "@/features/audio-settings/store";
import { NEWS_SANDBOX, parseNewsMessage } from "./messages";

const MIN_HEIGHT = 80;
const DEFAULT_HEIGHT = 360;
const MAX_HEIGHT = 640;

const day = (iso: string) => {
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? "" : formatDate(ms, { year: "numeric", month: "short", day: "numeric" });
};

function NewsFrame({ item, onClose }: { item: NewsItem; onClose: () => void }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState(DEFAULT_HEIGHT);
  useEffect(() => {
    let origin: string | null = null;
    try {
      origin = new URL(item.html_url).origin;
    } catch {
      /* not a URL: no message is accepted */
    }
    const onMessage = (e: MessageEvent) => {
      // Only this iframe's window; a sandboxed page's origin is "null".
      if (!frame.current || e.source !== frame.current.contentWindow) return;
      if (e.origin !== "null" && e.origin !== origin) return;
      const msg = parseNewsMessage(e.data);
      if (msg?.action === "close") onClose();
      else if (msg?.action === "resize") setHeight(Math.round(Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, msg.height))));
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [item.html_url, onClose]);
  return (
    <iframe
      ref={frame}
      className="eth-news__frame"
      title={item.title}
      src={item.html_url}
      sandbox={NEWS_SANDBOX}
      referrerPolicy="no-referrer"
      loading="eager"
      style={{ height }}
      data-testid="news-frame"
    />
  );
}

export function NewsViewer() {
  const t = useT();
  const current = useNews((s) => s.current);
  const replay = useNews((s) => s.replay);
  const more = useNews((s) => (s.current && !s.replay ? s.queue.filter((i) => i.id !== s.current!.id).length : 0));
  // #623: never over a guided tour (its overlay owns the screen). The item waits in the
  // store and shows once the tour ends; a tour started over it hides it until then.
  const touring = useTour((s) => s.tour !== null);
  const open = current !== null && !touring;
  // Escape wherever focus is in the app, not only in the dialog. Keys pressed inside the page
  // stay in its frame: the page can post `close` itself.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeNews();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);
  return (
    <Dialog
      open={open}
      onClose={closeNews}
      className="eth-news"
      title={
        <div className="eth-news__header">
          <span className="eth-news__title">{current?.title}</span>
          {current && <span className="eth-news__date">{day(current.published_at)}</span>}
          <IconButton size="sm" tone="ghost" label={t("common.close")} icon={<X aria-hidden />} onClick={closeNews} data-testid="news-close-x" />
        </div>
      }
      footer={
        <>
          {more > 0 && !replay && (
            <Button size="sm" onClick={nextNews} data-testid="news-next">
              {t("news.viewer.next", { count: more })}
            </Button>
          )}
          <Button size="sm" tone="accent" onClick={closeNews} data-testid="news-close">
            {t("common.close")}
          </Button>
        </>
      }
    >
      {current && (
        <div className="eth-news__body" data-testid="news-viewer">
          <NewsFrame key={current.html_url} item={current} onClose={closeNews} />
        </div>
      )}
    </Dialog>
  );
}

export function NewsList() {
  const t = useT();
  const open = useNews((s) => s.listOpen);
  const items = useNews((s) => s.items);
  const loading = useNews((s) => s.loading);
  const error = useNews((s) => s.error);
  const enabled = useNews((s) => s.enabled);
  return (
    <Dialog
      open={open}
      onClose={closeNewsList}
      title={t("news.list.title")}
      className="eth-news-list"
      footer={<Button onClick={closeNewsList}>{t("common.close")}</Button>}
    >
      {open && (
        <div className="eth-news-list__body" data-testid="news-list">
          {!enabled ? (
            <p className="eth-settings__hint">
              {t("news.list.off")}{" "}
              <Button
                size="sm"
                tone="ghost"
                onClick={() => {
                  closeNewsList();
                  openSettings("privacy");
                }}
              >
                {t("news.list.privacy")}
              </Button>
            </p>
          ) : loading && !items ? (
            <p className="eth-settings__hint">{t("news.list.loading")}</p>
          ) : error ? (
            <p className="eth-settings__hint" role="status">
              {t("news.list.error", { error })}
            </p>
          ) : !items || items.length === 0 ? (
            <p className="eth-settings__hint">{t("news.list.empty")}</p>
          ) : (
            <ul className="eth-news-list__items">
              {items.map((item) => (
                <li key={item.id}>
                  <button type="button" className="eth-news-list__item" onClick={() => showNewsItem(item)}>
                    <span className="eth-news-list__title">{item.title}</span>
                    <span className="eth-news__date">{day(item.published_at)}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </Dialog>
  );
}
