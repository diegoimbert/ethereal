/**
 * The mobile transport (#522): a thumb-reach bar at the bottom of the screen. The same
 * commands as the desktop transport bar (`features/transport-bar`), with touch-sized
 * buttons: stop, play/pause, record, loop, metronome, plus the position, the tempo and the
 * follow-playhead toggle on a slim status row above (one row in phone landscape).
 */
import { Circle, LocateFixed, Pause, Play, Repeat, Rows3, SlidersVertical, Square, Timer } from "lucide-react";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useTimelineView } from "@/timeline";
import { stopButton } from "@/features/time-edits/marker";
import { PositionDisplay } from "@/features/transport-bar";
import { useEngineCommands } from "@/features/transport-bar/engine";
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
  // The browser has no audio or MIDI inputs: no recording there (#717), like the desktop bar.
  const noRecording = useRecordingUnsupported(transport ?? null, !disabled);
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
          disabled={disabled}
          onClick={() => transport && stopButton(transport)}
        >
          <Square aria-hidden />
        </Button>
        <Button
          tone="ghost"
          aria-label={playing ? t("mobile.transport.pause") : t("mobile.transport.play")}
          active={playing}
          className="eth-mdock__btn eth-mdock__play"
          disabled={disabled}
          onClick={() => void send(cmd("Transport", { type: playing ? "Stop" : "Play" }))}
        >
          {playing ? <Pause aria-hidden /> : <Play aria-hidden />}
        </Button>
        <Button
          tone="ghost"
          aria-label={t("mobile.transport.record")}
          active={recording}
          className="eth-mdock__btn eth-mdock__record"
          title={noRecording ? unsupportedTooltip() : undefined}
          disabled={disabled || noRecording}
          onClick={() => void send(cmd("Recording", { type: "SetRecording", enabled: !recording }))}
        >
          <Circle aria-hidden />
        </Button>
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
