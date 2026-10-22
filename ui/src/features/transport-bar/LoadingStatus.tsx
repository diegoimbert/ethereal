import { LoaderCircle } from "lucide-react";
import type { LoadProgress } from "@/generated";
import { useT } from "@/i18n";
import { loadFraction, loadingText } from "./loading";

/**
 * The load's progress in the transport bar: a spinner, what it is doing and a bar (screen
 * readers hear it). The one loading indicator (#301); compact, the spinner and the bar alone.
 */
export function LoadingStatus({ progress, compact }: { progress: LoadProgress; compact?: boolean }) {
  const t = useT();
  const text = loadingText(progress);
  const pct = Math.round(loadFraction(progress) * 100);
  return (
    <span className="eth-tb__loading" role="status" data-testid="project-loading" title={t("error.project.loading")}>
      <LoaderCircle className="eth-tb__loading-spin" aria-hidden />
      {!compact && <span className="eth-tb__loading-text">{text}</span>}
      <span className="eth-tb__loading-bar" role="progressbar" aria-label={text} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
        <span className="eth-tb__loading-fill" style={{ ["--eth-tb-loading" as string]: String(pct / 100) }} />
      </span>
    </span>
  );
}
