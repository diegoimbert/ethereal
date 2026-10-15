// #597: the engine didn't start (no cross-origin isolation, no AudioWorklet, the wasm didn't
// load, a worker failed): shown in place of the editor, which would stay blank, with the
// reason, "retry" and "reload". A part of the app that failed to load (a page left open
// across a deploy) says a new version is out instead.
import { RefreshCw, TriangleAlert } from "lucide-react";
import { Button } from "@/kit";
import { isChunkLoadError } from "@/app/crashRecovery";
import { useT } from "@/i18n";
import { StartupScreen } from "./StartupScreen";

export interface EngineErrorScreenProps {
  error: unknown;
  /** Start the engine again (absent: only "reload"). */
  onRetry?: () => void;
  onReload?: () => void;
}

/** The engine's reason, as it reported it (English), or null when it gave none. */
function engineErrorText(error: unknown): string | null {
  const message = error instanceof Error ? error.message : typeof error === "string" ? error : "";
  return message.trim() || null;
}

export function EngineErrorScreen({ error, onRetry, onReload = () => window.location.reload() }: EngineErrorScreenProps) {
  const t = useT();
  if (isChunkLoadError(error)) {
    return (
      <StartupScreen
        testId="engine-error"
        icon={<RefreshCw />}
        title={t("startup.new_version")}
        actions={
          <Button tone="accent" size="lg" onClick={onReload}>
            {t("startup.reload")}
          </Button>
        }
      >
        <p className="eth-startup__text">{t("startup.new_version_hint")}</p>
      </StartupScreen>
    );
  }
  return (
    <StartupScreen
      testId="engine-error"
      icon={<TriangleAlert />}
      title={t("startup.engine_failed")}
      actions={
        <>
          {onRetry && (
            <Button tone="accent" size="lg" onClick={onRetry}>
              {t("startup.retry")}
            </Button>
          )}
          <Button size="lg" tone={onRetry ? "ghost" : "accent"} onClick={onReload}>
            {t("startup.reload")}
          </Button>
        </>
      }
    >
      <p className="eth-startup__text">{t("startup.engine_failed_hint")}</p>
      <pre className="eth-startup__detail" role="alert" data-i18n-exempt={engineErrorText(error) ? true : undefined}>
        {engineErrorText(error) ?? t("startup.unknown_error")}
      </pre>
    </StartupScreen>
  );
}
