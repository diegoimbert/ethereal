import type { RenderingState } from "@/generated";
import { useT } from "@/i18n";
import { renderPercent, renderingText } from "./rendering";

/**
 * A render's progress in the transport bar (the live engine is paused meanwhile, and play hears
 * what is rendered so far): its percent and a bar, with the explanation as a tooltip.
 */
export function RenderingStatus({ state, compact }: { state: RenderingState; compact?: boolean }) {
  const t = useT();
  const text = renderingText(state);
  const pct = renderPercent(state);
  const title = t(state.preview ? "shell.transport.rendering_title" : "shell.transport.rendering_no_preview_title");
  return (
    <span className="eth-tb__loading eth-tb__rendering" role="status" data-testid="render-preview" title={title}>
      {!compact && <span className="eth-tb__loading-text">{text}</span>}
      <span className="eth-tb__loading-bar" role="progressbar" aria-label={text} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
        <span className="eth-tb__loading-fill" style={{ ["--eth-tb-loading" as string]: String(pct / 100) }} />
      </span>
    </span>
  );
}
