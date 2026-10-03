// The web join landing (docs/SHARING.md §5): `apps/web` shows it for `/join/...` before the
// engine boots, so opening an invite costs no wasm download until the user picks the browser.
import "./landing.css";
import { Download, ExternalLink, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { Button, Toggle } from "@/kit";
import { APP_OPEN_TIMEOUT_MS, DOWNLOAD_URL, prefersBrowser, setPrefersBrowser, type JoinRoute } from "./landing";

export interface JoinLandingProps {
  route: JoinRoute;
  /** Boot the web app and open the invite. */
  onContinue(): void;
  /** Boot the web app without the invite (a damaged link). */
  onOpenApp(): void;
  /** Navigate to the deep link (injectable for tests). */
  openDeepLink?: (url: string) => void;
}

export function JoinLanding({ route, onContinue, onOpenApp, openDeepLink }: JoinLandingProps) {
  const [remember, setRemember] = useState(prefersBrowser);
  const [tried, setTried] = useState(false);
  const [offerDownload, setOfferDownload] = useState(false);

  // The app didn't take over (the page is still visible): it may not be installed.
  useEffect(() => {
    if (!tried) return;
    const t = setTimeout(() => {
      if (document.visibilityState === "visible") setOfferDownload(true);
    }, APP_OPEN_TIMEOUT_MS);
    return () => clearTimeout(t);
  }, [tried]);

  const openApp = () => {
    setOfferDownload(false);
    setTried(true);
    (openDeepLink ?? ((u: string) => window.location.assign(u)))(route.deepLink);
  };

  const continueInBrowser = () => {
    setPrefersBrowser(remember);
    onContinue();
  };

  return (
    <main className="eth-landing" data-testid="join-landing">
      <section className="eth-landing__card" aria-labelledby="eth-landing-title">
        <span className="eth-landing__icon" aria-hidden>
          <Users />
        </span>
        <p className="eth-landing__brand">Ethereal</p>
        {route.problem ? (
          <>
            <h1 id="eth-landing-title" className="eth-landing__title">
              This invite can't be opened
            </h1>
            <p className="eth-landing__error" role="alert">
              {route.problem}
            </p>
            <div className="eth-landing__actions">
              <Button tone="accent" size="lg" onClick={onOpenApp}>
                Open Ethereal
              </Button>
            </div>
          </>
        ) : (
          <>
            <h1 id="eth-landing-title" className="eth-landing__title">
              You've been invited to an Ethereal project
            </h1>
            <p className="eth-landing__text">Join from the desktop app, or right here in your browser.</p>
            <div className="eth-landing__actions">
              <Button tone="accent" size="lg" onClick={openApp}>
                <ExternalLink aria-hidden className="eth-landing__btn-icon" />
                Open in the app
              </Button>
              <Button size="lg" onClick={continueInBrowser} data-testid="join-continue">
                Continue in browser
              </Button>
            </div>
            {offerDownload && (
              <p className="eth-landing__download" role="status">
                Don't have the app?{" "}
                <a href={DOWNLOAD_URL} target="_blank" rel="noreferrer">
                  <Download aria-hidden className="eth-landing__link-icon" />
                  Download it
                </a>
              </p>
            )}
            <Toggle size="sm" className="eth-landing__remember" checked={remember} onChange={setRemember} label="Always continue in browser" />
          </>
        )}
      </section>
    </main>
  );
}
