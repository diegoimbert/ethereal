// Tells the user when the engine stops responding (playhead frozen, commands queued while
// audio keeps playing), and why when the engine knows.
//
// Two signals, because neither is enough alone:
// - Meter frames arrive ~30 times a second whenever the engine runs; a silence means it is
//   stuck (or that nothing reaches the UI). On desktop this is the only live signal: the
//   engine's messages reach the webview through the app's main thread, often the very
//   thing that is blocked.
// - The engine's watchdog sends `Engine::Unresponsive` (with a reason, e.g. "Analog Lab V
//   is busy (opening its editor)") and `Engine::Responsive` (with how long it lasted).
//   They arrive live when only the engine's controller thread is stuck, else right after.
import type { EngineEvent } from "@/generated";
import { t } from "@/i18n";

/** No meter frame for this long (while armed and visible) = unresponsive. */
export const SILENCE_MS = 2500;
/** After frames resume, wait this long for the engine's `Responsive` (it has the reason). */
export const SETTLE_MS = 1500;

export interface HealthUi {
  /** Show the ongoing warning; returns its id. */
  warn(message: string): number;
  dismiss(id: number): void;
  /** The one-off summary once it is over. */
  info(message: string): void;
}

type Stall = {
  /** When the UI noticed it (`now()` time). */
  since: number;
  /** The engine's reason, once known. */
  reason: string | null;
  /** The engine's own measure of the stall, once over. */
  engineMs: number | null;
  notice: number | null;
  /** Frames are flowing again (since then): waiting for the engine's `Responsive`. */
  resumedAt: number | null;
};

export class EngineHealthWatcher {
  /** Last meter frame; `null` = not armed (no frame yet, or the page was hidden). */
  private lastFrame: number | null = null;
  private stall: Stall | null = null;

  constructor(
    private readonly ui: HealthUi,
    private readonly now: () => number = () => performance.now(),
  ) {}

  /** A meter frame arrived. */
  frame(): void {
    const t = this.now();
    this.lastFrame = t;
    if (this.stall && this.stall.resumedAt === null) this.stall.resumedAt = t;
  }

  /** An engine event (only `Unresponsive` / `Responsive` matter). */
  event(e: EngineEvent): void {
    if (e.type === "Unresponsive") {
      const t = this.now();
      if (!this.stall) this.stall = { since: t - e.stalled_ms, reason: null, engineMs: null, notice: null, resumedAt: null };
      if (e.reason && e.reason !== this.stall.reason) {
        this.stall.reason = e.reason;
        this.showWarning();
      } else if (this.stall.notice === null) {
        this.showWarning();
      }
    } else if (e.type === "Responsive") {
      if (!this.stall) return; // the UI never saw it start (e.g. reloaded meanwhile)
      this.stall.reason = e.reason ?? this.stall.reason;
      this.stall.engineMs = e.stalled_ms;
      this.finish();
    }
  }

  /** The page was hidden: frames may legitimately stop (or timers lag); re-arm on the next frame. */
  hidden(): void {
    this.lastFrame = null;
  }

  /** Periodic check (a few times a second). */
  check(): void {
    const t = this.now();
    if (this.stall) {
      if (this.stall.resumedAt !== null && t - this.stall.resumedAt >= SETTLE_MS) this.finish();
      return;
    }
    if (this.lastFrame !== null && t - this.lastFrame >= SILENCE_MS) {
      this.stall = { since: this.lastFrame, reason: null, engineMs: null, notice: null, resumedAt: null };
      this.showWarning();
    }
  }

  private showWarning(): void {
    const s = this.stall!;
    if (s.notice !== null) this.ui.dismiss(s.notice);
    s.notice = this.ui.warn(s.reason ? t("shell.engine_health.stalled_reason", { reason: s.reason }) : t("shell.engine_health.stalled"));
  }

  private finish(): void {
    const s = this.stall!;
    this.stall = null;
    if (s.notice !== null) this.ui.dismiss(s.notice);
    const ms = s.engineMs ?? (s.resumedAt ?? this.now()) - s.since;
    const secs = Math.max(1, Math.round(ms / 1000));
    this.ui.info(s.reason ? t("shell.engine_health.recovered_reason", { secs, reason: s.reason }) : t("shell.engine_health.recovered", { secs }));
  }
}
