/**
 * A WebCLAP plugin's own web ui, in its device window: a sandboxed frame the host app fills
 * (`WebclapApi.mountUi`: no access to the app, wired to the plugin by messages). Plugins
 * without a web ui show the generic parameter panel instead (`DeviceEditor`).
 */
import { useEffect, useRef, useState } from "react";
import type { Device } from "@/generated";
import { useT } from "@/i18n";
import type { WebclapApi } from "./api";
import "./webclap.css";

export function WebclapEditor({ device, api }: { device: Device; api: WebclapApi }) {
  const t = useT();
  const frame = useRef<HTMLIFrameElement>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const pluginId = device.kind.type === "Plugin" ? device.kind.plugin.plugin_id : "";
  useEffect(() => {
    const iframe = frame.current;
    if (!iframe) return;
    let close: (() => void) | null = null;
    let alive = true;
    setError(null);
    setReady(false);
    api.mountUi(device.id, pluginId, iframe).then(
      (c) => {
        if (alive) {
          close = c;
          setReady(true);
        } else c();
      },
      (e: unknown) => alive && setError(e instanceof Error ? e.message : String(e)),
    );
    return () => {
      alive = false;
      close?.();
    };
  }, [api, device.id, pluginId]);
  return (
    <div className="eth-webclap-editor" data-webclap-editor={device.id}>
      {error ? (
        <p className="eth-webclap__error" role="alert">
          {t("webclap.editor.failed", { error })}
        </p>
      ) : (
        !ready && <p className="eth-webclap__note">{t("webclap.editor.loading")}</p>
      )}
      <iframe
        ref={frame}
        className="eth-webclap-editor__frame"
        title={t("webclap.editor.frame", { name: device.name })}
        sandbox="allow-scripts"
        hidden={!!error}
      />
    </div>
  );
}
