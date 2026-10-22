/**
 * The mobile transport: a thumb-reach bar at the bottom of the screen. The same
 * commands as the desktop transport bar (`features/transport-bar`), with touch-sized
 * buttons: stop, play/pause, record, loop, metronome, plus the position, the tempo and the
 * follow-playhead toggle on a slim status row above (one row in phone landscape). A read-only
 * view (`state/viewer.ts`) has no record or metronome: stop, play, loop and the view switch.
 */
import { Circle, LocateFixed, Pause, Play, Repeat, Rows3, SlidersVertical, Square, Timer } from "lucide-react";
import { Button } from "@/kit";
import { useProjectStore, useReadOnly } from "@/state";
import { cmd } from "@/transport";
import { useTimelineView } from "@/timeline";
import { stopButton } from "@/features/time-edits/marker";
import { PositionDisplay } from "@/features/transport-bar";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { useProjectLoading } from "@/features/transport-bar/loading";
import { useRendering } from "@/features/transport-bar/rendering";
import { formatBpm, formatSignature } from "@/features/transport-bar/format";
import { arrangementView } from "@/features/arrangement/uiStore";
import { unsupportedTooltip, useRecordingUnsupported } from "@/features/recording/support";
import { useMobileView } from "./viewState";
import { useT } from "@/i18n";

export function TransportDock() {
  const t = useT();
  const { transport, send } = useEngineCommands();
  const state = useProjectStore((s) => s.transport);
  const hasProject = useProjectStore((s) => s.project !== null);
  const follow = useTimelineView(arrangementView, (s) => s.followPlayhead);
  const disabled = !transport || !hasProject;
  // Still loading: no play, stop or record yet (the rest works).
  const loading = useProjectLoading() !== null;
  const readOnly = useReadOnly();
  // A render runs: play hears what is rendered so far (if anything), record waits.
  const render = useRendering();
  const noPreview = !!render && !render.preview;
  // The browser has no audio or MIDI inputs: no recording there, like the desktop bar.
  const noRecording = useRecordingUnsupported(transport ?? null, !disabled && !readOnly);
  const playing = state?.playing ?? false;
  const recording = state?.recording ?? false;
  const loop = state?.loop_enabled ?? false;
  const metronome = state?.metronome ?? false;
  const bpm = state?.bpm ?? 120;
  const signature = state?.time_signature ?? { numerator: 4, denominator: 4 };

  return (
    <div className="eth-mdock" data-testid="mobile-transport">
      <div className="eth-mdock__status">
        <PositionDisplay />
        <span
          className="eth-mdock__tempo"
          data-testid="mobile-tempo"
          aria-label={t("mobile.transport.tempo", { bpm: formatBpm(bpm), signature: formatSignature(signature) })}
        >
          <span className="eth-mdock__bpm">{formatBpm(bpm)}</span>
          <span className="eth-mdock__unit">bpm</span>
          <span className="eth-mdock__sig">{formatSignature(signature)}</span>
        </span>
        <Button
          tone="ghost"
          aria-label={t("mobile.transport.follow")}
          className="eth-mdock__btn eth-mdock__btn--small"
          active={follow}
          disabled={disabled}
          onClick={() => arrangementView.getState().setFollowPlayhead(!follow)}
        >
          <LocateFixed aria-hidden />
        </Button>
      </div>
      <div className="eth-mdock__main" role="toolbar" aria-label={t("mobile.transport.label")}>
        <Button
          tone="ghost"
          aria-label={t("mobile.transport.stop")}
          data-testid="transport-stop"
          className="eth-mdock__btn"
          title={loading ? t("error.project.loading") : undefined}
          disabled={disabled || loading}
          onClick={() => transport && stopButton(transport)}
        >
          <Square aria-hidden />
        </Button>
        <Button
          tone="ghost"
          aria-label={playing ? t("mobile.transport.pause") : t("mobile.transport.play")}
          active={playing}
          className="eth-mdock__btn eth-mdock__play"
          title={loading ? t("error.project.loading") : noPreview ? t("error.render.no_preview") : undefined}
          disabled={disabled || loading || (noPreview && !playing)}
          onClick={() => void send(cmd("Transport", { type: playing ? "Stop" : "Play" }))}
        >
          {playing ? <Pause aria-hidden /> : <Play aria-hidden />}
        </Button>
        {!readOnly && (
          <Button
            tone="ghost"
            aria-label={t("mobile.transport.record")}
            active={recording}
            className="eth-mdock__btn eth-mdock__record"
            title={noRecording ? unsupportedTooltip() : loading ? t("error.project.loading") : render ? t("error.render.running") : undefined}
            disabled={disabled || noRecording || loading || (!!render && !recording)}
            onClick={() => void send(cmd("Recording", { type: "SetRecording", enabled: !recording }))}
          >
            <Circle aria-hidden />
          </Button>
        )}
        <Button
          tone="ghost"
          aria-label={t("mobile.transport.loop")}
          active={loop}
          className="eth-mdock__btn"
          disabled={disabled}
          onClick={() => void send(cmd("Transport", { type: "SetLoopEnabled", enabled: !loop }))}
        >
          <Repeat aria-hidden />
        </Button>
        {!readOnly && (
          <Button
            tone="ghost"
            aria-label={t("mobile.transport.metronome")}
            active={metronome}
            className="eth-mdock__btn"
            disabled={disabled}
            onClick={() => void send(cmd("Transport", { type: "SetMetronome", enabled: !metronome }))}
          >
            <Timer aria-hidden />
          </Button>
        )}
        <ViewSwitch />
      </div>
    </div>
  );
}

/** Tracks (the arrangement) or the mixer. */
function ViewSwitch() {
  const t = useT();
  const view = useMobileView((s) => s.view);
  const other = view === "mixer" ? "tracks" : "mixer";
  return (
    <Button
      tone="ghost"
      aria-label={other === "mixer" ? t("mobile.transport.show_mixer") : t("mobile.transport.show_tracks")}
      className="eth-mdock__btn eth-mdock__view"
      data-testid="view-switch"
      onClick={() => useMobileView.setState({ view: other })}
    >
      {other === "mixer" ? <SlidersVertical aria-hidden /> : <Rows3 aria-hidden />}
    </Button>
  );
}
