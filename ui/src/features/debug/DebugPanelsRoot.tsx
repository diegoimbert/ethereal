import { Component, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { FloatingWindow, Toggle } from "@/kit";
import { t as translate, useT } from "@/i18n";
import { closeDebugPanel, debugWindowId, setDebugPanelOpen, useDebugPanels, type DebugPanel } from "./registry";
import "./debug.css";

/**
 * The open debug panels, each in a kit `FloatingWindow` (moved, resized, kept on
 * screen, geometry remembered). Mounted once by the app shell.
 */
export function DebugPanelsRoot() {
  const panels = useDebugPanels((s) => s.panels);
  const open = useDebugPanels((s) => s.open);
  const shown = panels.filter((p) => open.includes(p.id));
  if (shown.length === 0) return null;
  return createPortal(
    <div className="eth-debug-panels" data-testid="debug-panels">
      {shown.map((p) => (
        <DebugPanelWindow key={p.id} panel={p} />
      ))}
    </div>,
    document.body,
  );
}

function DebugPanelWindow({ panel }: { panel: DebugPanel }) {
  const t = useT();
  return (
    <FloatingWindow
      id={debugWindowId(panel.id)}
      label={t("debug.window.label", { title: panel.title })}
      closeLabel={t("debug.window.close", { title: panel.title })}
      title={
        <>
          <span className="eth-debug-panel__title">{panel.title}</span>
          <span className="eth-debug-panel__kind">{t("debug.window.kind")}</span>
        </>
      }
      defaultWidth={panel.defaultSize?.width}
      defaultHeight={panel.defaultSize?.height}
      onClose={() => closeDebugPanel(panel.id)}
      className="eth-debug-panel"
      bodyClassName="eth-debug-panel__body"
      data-debug-panel={panel.id}
    >
      <PanelBoundary title={panel.title}>{panel.render()}</PanelBoundary>
    </FloatingWindow>
  );
}

/** A panel that throws shows the error instead of taking the app down. */
class PanelBoundary extends Component<{ title: string; children: ReactNode }, { error: Error | null }> {
  override state: { error: Error | null } = { error: null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  override render() {
    if (!this.state.error) return this.props.children;
    return (
      <p className="eth-debug-panel__error" role="alert">
        {translate("debug.window.failed", { title: this.props.title, error: this.state.error.message })}
      </p>
    );
  }
}

/** Settings › Advanced: every registered debug panel, with its open switch. */
export function DebugPanelsSettings({ onOpen }: { onOpen?: () => void }) {
  const t = useT();
  const panels = useDebugPanels((s) => s.panels);
  const open = useDebugPanels((s) => s.open);
  if (panels.length === 0) return null;
  return (
    <section className="eth-settings__section" aria-label={t("debug.settings.title")} data-testid="settings-debug-panels">
      <h3 className="eth-settings__heading">{t("debug.settings.title")}</h3>
      <p className="eth-settings__hint">{t("debug.settings.hint")}</p>
      {panels.map((p) => (
        <Toggle
          key={p.id}
          size="sm"
          checked={open.includes(p.id)}
          label={p.title}
          onChange={(v) => {
            setDebugPanelOpen(p.id, v);
            if (v) onOpen?.();
          }}
        />
      ))}
    </section>
  );
}
