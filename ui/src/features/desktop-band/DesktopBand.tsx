// The web version's "Open on desktop" band: a slim, persistent strip above the app shell
// (apps/web mounts it on desktop machines only; never in the desktop app itself).
//
// "Open on desktop" follows `ethereal://open`, which launches the desktop app or brings it
// forward (the shell's deep-link handler; the UI ignores the link). A web project lives in the
// browser's storage, so nothing is handed over.
//
// The close button hides the band until the page is reloaded (in memory only, never stored).
import "./desktopBand.css";
import { ExternalLink, X } from "lucide-react";
import { useSyncExternalStore, type ReactNode } from "react";
import { useT } from "@/i18n";
import { Button, IconButton, Logo } from "@/kit";
import { isTauri } from "@/transport";

/** The band's message (a message key). */
export const BAND_TEXT = "shell.desktop_band.text";

/** The desktop app's deep link: launches it or brings it forward. */
export const OPEN_APP_LINK = "ethereal://open";

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
}

export function DesktopBand({ openDeepLink }: DesktopBandProps) {
  const t = useT();
  const closed = useSyncExternalStore(subscribe, isDismissed, isDismissed);

  if (isTauri() || closed) return null;

  const open = () => (openDeepLink ?? ((u: string) => window.location.assign(u)))(OPEN_APP_LINK);

  return (
    <aside className="eth-desktop-band" data-testid="desktop-band" aria-label={t("shell.desktop_band.label")}>
      <Logo />
      <span className="eth-desktop-band__text">{t(BAND_TEXT)}</span>
      <Button size="sm" tone="accent" className="eth-desktop-band__open" onClick={open}>
        <ExternalLink aria-hidden />
        {t("shell.desktop_band.open")}
      </Button>
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
