/**
 * Error boundaries around the editor's regions (#590).
 *
 * A render error in one region (the arrangement, a pane, a top-bar control, a dialog root)
 * used to unmount the whole React root: a blank window. Each region now has a
 * `PanelBoundary`: the region shows "<name> crashed" with "reload panel" (a fresh mount)
 * and "reload", and the rest of the editor keeps working. The project is safe either way:
 * the engine holds the document, and "reload" saves before reloading.
 *
 * `variant="app"` is the last resort around the whole app (and the lazy mobile shell, whose
 * chunk can fail to load right after a deploy): a full-window card with the same actions.
 */

import { Component, useContext, type ErrorInfo, type ReactNode } from "react";
import { useT, type MessageKey } from "@/i18n";
import { Button } from "@/kit";
import { TransportContext } from "@/transport";
import { isChunkLoadError, saveAndReload } from "./crashRecovery";
import "./PanelBoundary.css";

/** The error message shown in a crashed panel, at most this many characters. */
const DETAIL_MAX = 300;

/** The regions with a boundary: id (logs, telemetry, `data-panel`) → its name in a sentence. */
const PANEL_NAMES = {
  app: "shell.panel.app",
  arrangement: "shell.panel.arrangement",
  "command palette": "shell.panel.command_palette",
  "debug panels": "shell.panel.debug_panels",
  "device windows": "shell.panel.device_windows",
  documentation: "shell.panel.documentation",
  "drawer shortcut": "shell.panel.drawer_shortcut",
  editor: "shell.panel.editor",
  "engine connection": "shell.panel.engine_connection",
  export: "shell.panel.export",
  import: "shell.panel.import",
  "import resolver": "shell.panel.import_resolver",
  inspector: "shell.panel.inspector",
  keymap: "shell.panel.keymap",
  library: "shell.panel.library",
  midi: "shell.panel.midi",
  history: "shell.panel.history",
  ai: "shell.panel.ai",
  media: "shell.panel.media",
  metronome: "shell.panel.metronome",
  "midi learn": "shell.panel.midi_learn",
  "mobile editor": "shell.panel.mobile_editor",
  "project menu": "shell.panel.project_menu",
  quit: "shell.panel.quit",
  rail: "shell.panel.rail",
  "recording controls": "shell.panel.recording_controls",
  settings: "shell.panel.settings",
  tour: "shell.panel.tour",
  "transport bar": "shell.panel.transport_bar",
  versions: "shell.panel.versions",
  workspace: "shell.panel.workspace",
} satisfies Record<string, MessageKey>;

export type PanelName = keyof typeof PANEL_NAMES;

export interface PanelBoundaryProps {
  /** What crashed: a stable id (logs, telemetry, `data-panel`), shown through `PANEL_NAMES`. */
  name: PanelName;
  /** "panel" (default): in place of the region; "overlay": a floating card (dialog roots);
   * "app": the whole window. */
  variant?: "panel" | "overlay" | "app";
  children: ReactNode;
}

interface State {
  error: Error | null;
  /** Bumped by "reload panel": the children mount afresh. */
  generation: number;
}

export class PanelBoundary extends Component<PanelBoundaryProps, State> {
  override state: State = { error: null, generation: 0 };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error(`[ethereal] ${this.props.name} crashed:`, error, info.componentStack);
  }

  private readonly retry = () => this.setState((s) => ({ error: null, generation: s.generation + 1 }));

  override render(): ReactNode {
    const { error, generation } = this.state;
    if (!error) return <Generation key={generation}>{this.props.children}</Generation>;
    return <CrashCard name={this.props.name} variant={this.props.variant ?? "panel"} error={error} onRetry={this.retry} />;
  }
}

function Generation({ children }: { children: ReactNode }) {
  return children;
}

function CrashCard({ name, variant, error, onRetry }: { name: PanelName; variant: "panel" | "overlay" | "app"; error: Error; onRetry: () => void }) {
  const t = useT();
  const shown = Object.hasOwn(PANEL_NAMES, name) ? t(PANEL_NAMES[name]) : t("shell.crash.panel_other", { name });
  const transport = useContext(TransportContext)?.transport ?? null;
  const chunk = isChunkLoadError(error);
  const app = variant === "app";
  return (
    <div className={`eth-crash eth-crash--${variant}`} role="alert" data-testid="panel-crash" data-panel={name}>
      <div className="eth-crash__card">
        <p className="eth-crash__title">{chunk ? t("shell.crash.load_failed", { name: shown }) : t("shell.crash.crashed", { name: shown })}</p>
        <p className="eth-crash__hint">{chunk ? t("shell.crash.hint_chunk") : app ? t("shell.crash.hint_app") : t("shell.crash.hint_panel")}</p>
        {!chunk && <pre className="eth-crash__detail">{error.message.slice(0, DETAIL_MAX)}</pre>}
        <div className="eth-crash__actions">
          {!app && !chunk && (
            <Button size="sm" tone="accent" onClick={onRetry}>
              {t("shell.crash.reload_panel")}
            </Button>
          )}
          <Button size="sm" tone={app || chunk ? "accent" : "ghost"} onClick={() => void saveAndReload(transport)}>
            {t("shell.crash.reload")}
          </Button>
        </div>
      </div>
    </div>
  );
}
