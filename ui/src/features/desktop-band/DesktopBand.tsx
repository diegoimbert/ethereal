// The web version's "Open on desktop" band: a slim, persistent strip above the app shell
// (apps/web mounts it on desktop machines only; never in the desktop app itself).
//
// "Open on desktop" follows `ethereal://open`, which launches the desktop app or brings it
// forward (the shell's deep-link handler; the UI ignores the link). A web project lives in the
// browser's storage, so nothing is handed over. If the page still has the focus and is still
// visible a moment later, the app probably isn't installed: the band offers the download, when
// an extension serves a download page (`@/ext` "web.downloadUrl"; in a new tab so the project
// stays open).
//
// The close button hides the band until the page is reloaded (in memory only, never stored).
import "./desktopBand.css";
import { Download, ExternalLink, X } from "lucide-react";
import { useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import { DiscordLink } from "@/features/community";
import { Trans, useT } from "@/i18n";
import { Button, IconButton, Logo } from "@/kit";
import { isTauri } from "@/transport";
import { contributions } from "@/ext";

/** The desktop app's deep link: launches it or brings it forward. */
export const OPEN_APP_LINK = "ethereal://open";

/** How long the page waits for the app to take over before offering the download. */
export const APP_OPEN_TIMEOUT_MS = 1500;

/** The band's message (a message key). */
export const BAND_TEXT = "shell.desktop_band.text";

// Closed for this page load only: a reload shows the band again.
let dismissed = false;
const listeners = new Set<() => void>();
const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};
const isDismissed = () => dismissed;

/** Hide the band until the page is reloaded. */
export function dismissDesktopBand(): void {
  dismissed = true;
  listeners.forEach((l) => l());
}

/** Tests: a fresh page load (the band shows again). */
export function resetDesktopBandDismissal(): void {
  dismissed = false;
  listeners.forEach((l) => l());
}

export interface DesktopBandProps {
  /** Navigate to the deep link (injectable for tests). */
  openDeepLink?: (url: string) => void;
  /** Open a web link in a new tab (injectable for tests; default `openExternal`). */
  openLink?: (url: string) => void;
}

export function DesktopBand({ openDeepLink, openLink }: DesktopBandProps) {
  const t = useT();
  // Bumped on each click: restarts the "did the app take over?" check.
  const [attempt, setAttempt] = useState(0);
  // The download page an extension serves (`@/ext` "web.downloadUrl"), if any.
  const downloadUrl = contributions("web.downloadUrl")[0]?.() ?? null;
  const [offerDownload, setOfferDownload] = useState(false);

  useEffect(() => {
    if (attempt === 0) return;
    let left = false;
    const onLeave = () => {
      if (document.visibilityState === "hidden" || !document.hasFocus()) left = true;
    };
    window.addEventListener("blur", onLeave);
    document.addEventListener("visibilitychange", onLeave);
    const t = setTimeout(() => {
      if (!left && document.visibilityState === "visible" && downloadUrl) setOfferDownload(true);
    }, APP_OPEN_TIMEOUT_MS);
    return () => {
      clearTimeout(t);
      window.removeEventListener("blur", onLeave);
      document.removeEventListener("visibilitychange", onLeave);
    };
  }, [attempt, downloadUrl]);

  const closed = useSyncExternalStore(subscribe, isDismissed, isDismissed);

  if (isTauri() || closed) return null;

  const open = () => {
    setOfferDownload(false);
    setAttempt((n) => n + 1);
    (openDeepLink ?? ((u: string) => window.location.assign(u)))(OPEN_APP_LINK);
  };

  return (
    <aside className="eth-desktop-band" data-testid="desktop-band" aria-label={t("shell.desktop_band.label")}>
      <Logo />
      {offerDownload ? (
        <span className="eth-desktop-band__text" role="status">
          <Trans
            k="shell.desktop_band.no_app"
            values={{
              link: (chunks) => (
                <a className="eth-desktop-band__link" href={downloadUrl ?? undefined} target="_blank" rel="noreferrer">
                  <Download aria-hidden className="eth-desktop-band__link-icon" />
                  {chunks}
                </a>
              ),
            }}
          />
        </span>
      ) : (
        <span className="eth-desktop-band__text">{t(BAND_TEXT)}</span>
      )}
      <Button size="sm" tone="accent" className="eth-desktop-band__open" onClick={open}>
        <ExternalLink aria-hidden />
        {t("shell.desktop_band.open")}
      </Button>
      <DiscordLink className="eth-desktop-band__discord" open={openLink} />
      <IconButton size="sm" tone="ghost" className="eth-desktop-band__close" label={t("common.close")} icon={<X />} onClick={dismissDesktopBand} />
    </aside>
  );
}

/**
 * The app with the band above it (`show`: a desktop machine, decided by apps/web). Without the
 * band, the children render as they are. Closing the band keeps the frame (the app isn't
 * remounted); only the band goes.
 */
export function DesktopBandFrame({ show, children, ...band }: DesktopBandProps & { show: boolean; children: ReactNode }) {
  if (!show || isTauri()) return <>{children}</>;
  return (
    <div className="eth-web-frame">
      <DesktopBand {...band} />
      <div className="eth-web-frame__app">{children}</div>
    </div>
  );
}
