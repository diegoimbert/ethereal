import { useState } from "react";
import { LoaderCircle } from "lucide-react";
import type { ScanFailure } from "@/generated";
import { Badge, Button, Dialog, NumberField, openContextMenu } from "@/kit";
import { cmd, commandErrorText, isCommandFailed, type EngineTransport } from "@/transport";
import { t as tr, useT } from "@/i18n";
import { FORMAT_LABEL } from "./filter";
import { failureKey, usePluginStore } from "./pluginStore";

/** Default timeout of a retry: long enough for slow plugins (big sample libraries) that miss
 * the scanner's default 20 s. The retry dialog lets the user raise it (shell bundles with
 * hundreds of plugins can take many minutes). */
export const RETRY_TIMEOUT_MS = 90_000;
/** Bounds of the retry timeout the dialog accepts (s); the engine clamps to the same range. */
export const RETRY_TIMEOUT_MIN_S = 1;
export const RETRY_TIMEOUT_MAX_S = 3600;

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
  return e instanceof Error ? e.message : String(e);
}

/**
 * Retry one failed target with a longer timeout. The engine scans it in its own process and
 * replies when done; the row leaves the list if it loaded, else shows the new reason.
 */
async function retry(transport: EngineTransport, f: ScanFailure, timeoutMs: number): Promise<void> {
  const store = usePluginStore.getState();
  if (!f.format || failureKey(f) in store.retrying) return;
  store.setRetrying(f, true);
  try {
    const reply = await transport.send(cmd("Plugin", { type: "RetryScan", path: f.path, format: f.format, timeout_ms: timeoutMs }));
    if (reply.type === "ScanRetried") usePluginStore.getState().retried(f, reply.failure);
  } catch (e) {
    usePluginStore.getState().retried(f, { ...f, message: tr("plugins.failed.retry_failed", { error: errorText(e) }) });
  } finally {
    usePluginStore.getState().setRetrying(f, false);
  }
}

/** Settings → Plugins: the plugins that failed to load, each retryable from its context menu. */
export function FailedPluginsDialog({ transport, open, onClose }: { transport: EngineTransport; open: boolean; onClose: () => void }) {
  const t = useT();
  const failed = usePluginStore((s) => s.failed).filter((f) => f.path);
  const retrying = usePluginStore((s) => s.retrying);
  // The plugin whose retry is being set up (its timeout dialog is open), and that timeout.
  const [pending, setPending] = useState<ScanFailure | null>(null);
  const [timeoutS, setTimeoutS] = useState(RETRY_TIMEOUT_MS / 1000);
  const startRetry = () => {
    if (pending) void retry(transport, pending, timeoutS * 1000);
    setPending(null);
  };

  return (
    <>
      <Dialog
        open={open}
        onClose={onClose}
        title={t("plugins.failed.title")}
        className="eth-failed-plugins"
        footer={
          <Button size="sm" onClick={onClose}>
            {t("common.close")}
          </Button>
        }
      >
        {failed.length === 0 ? (
          <p className="eth-plugin-folders__note">{t("plugins.failed.none")}</p>
        ) : (
          <>
            <p className="eth-plugin-folders__note">{t("plugins.failed.hint")}</p>
            <ul className="eth-failed-plugins__list" aria-label={t("plugins.failed.list")}>
              {failed.map((f) => {
                const busy = failureKey(f) in retrying;
                return (
                  <li
                    key={failureKey(f)}
                    className="eth-failed-plugins__row"
                    aria-busy={busy || undefined}
                    data-busy={busy ? "" : undefined}
                    onContextMenu={(e) =>
                      openContextMenu(e, [
                        {
                          label: t("plugins.failed.retry_menu"),
                          disabled: busy || !f.format,
                          reason: busy ? t("plugins.failed.already_retrying") : t("plugins.failed.unknown_format"),
                          onSelect: () => setPending(f),
                        },
                      ])
                    }
                  >
                    <div className="eth-failed-plugins__head">
                      <span className="eth-plugin-folders__path eth-user-text" title={f.path}>
                        {f.path}
                      </span>
                      {f.format && <Badge>{FORMAT_LABEL[f.format]}</Badge>}
                    </div>
                    <span className="eth-failed-plugins__reason">
                      {busy ? (
                        <>
                          <LoaderCircle className="eth-failed-plugins__spin" aria-hidden /> {t("plugins.failed.retrying")}
                        </>
                      ) : (
                        f.message || t("plugins.failed.unknown_error")
                      )}
                    </span>
                  </li>
                );
              })}
            </ul>
          </>
        )}
      </Dialog>
      <Dialog
        open={pending !== null}
        onClose={() => setPending(null)}
        title={t("plugins.failed.retry_title")}
        footer={
          <>
            <Button size="sm" tone="ghost" onClick={() => setPending(null)}>
              {t("plugins.failed.cancel")}
            </Button>
            <Button size="sm" tone="accent" onClick={startRetry}>
              {t("plugins.failed.retry")}
            </Button>
          </>
        }
      >
        <p className="eth-plugin-folders__note eth-failed-plugins__retry-path eth-user-text" title={pending?.path}>
          {pending?.path}
        </p>
        <label className="eth-failed-plugins__timeout">
          <span>{t("plugins.failed.timeout")}</span>
          <NumberField
            aria-label={t("plugins.failed.timeout")}
            value={timeoutS}
            onChange={setTimeoutS}
            min={RETRY_TIMEOUT_MIN_S}
            max={RETRY_TIMEOUT_MAX_S}
            step={10}
            unit="s"
            autoFocus
          />
        </label>
        <p className="eth-plugin-folders__note">{t("plugins.failed.shell_hint")}</p>
      </Dialog>
    </>
  );
}
