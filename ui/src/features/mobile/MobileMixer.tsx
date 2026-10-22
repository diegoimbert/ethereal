/**
 * The mixer on touch screens (the design's §2 "Mixer view"): a row of channel strips
 * that scrolls sideways, the master pinned on the right. Each strip: the name, pan (the
 * header's knob), a tall fader with the track's meter, the level in dB, and mute / solo / arm.
 * The fader and knob drag relative to where they were (never jumping); moving the finger
 * sideways while dragging slows them down (fine mode). Double-tap resets. The desktop has no
 * mixer view (the track headers took it over), so this is the mobile UI's own, built from the
 * same commands as the headers. In a read-only view (`state/viewer.ts`) the fader and pan are
 * disabled (kit controls), there is no arm, and Mute / Solo work where the view says they do
 * (as the headers', `listenBlockedReason`).
 */
import { Circle, Headphones, Volume2, VolumeX } from "lucide-react";
import type { Track } from "@/generated";
import { Fader, Meter } from "@/kit";
import { formatDb } from "@/domain/paramScale";
import { trackSoloCommand } from "@/domain/soloCommands";
import { muteOverriddenBySolo } from "@/domain/soloPlan";
import { listenBlockedReason, useProjectStore, useTrackMeter, useTracksOrdered, useViewer } from "@/state";
import { cmd } from "@/transport";
import { useGestureSender } from "@/features/devices/gesture";
import { sendEdit } from "@/features/arrangement/context";
import { dbToFader, faderToDb } from "@/features/arrangement/faderLaw";
import { HeaderPan } from "@/features/arrangement/HeaderPan";
import { colorCss } from "@/features/arrangement/helpers";
import { useTrackDisplayName } from "@/features/arrangement/trackDisplayName";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { useT } from "@/i18n";

/** Strips shown: every track but the master (pinned) and VCAs (no audio of their own). */
const stripTracks = (tracks: ReadonlyArray<Track>) => tracks.filter((t) => t.kind !== "Master" && t.kind !== "Vca");

export function MobileMixer() {
  const t = useT();
  const tracks = useTracksOrdered();
  const master = tracks.find((tr) => tr.kind === "Master");
  return (
    <section className="eth-mmixer" aria-label={t("mobile.mixer.label")} data-testid="mobile-mixer">
      <div className="eth-mmixer__strips" role="list">
        {stripTracks(tracks).map((tr) => (
          <Strip key={tr.id} track={tr} />
        ))}
        {stripTracks(tracks).length === 0 && <p className="eth-mmixer__empty">{t("mobile.mixer.empty")}</p>}
      </div>
      {master && (
        <div className="eth-mmixer__master" role="list">
          <Strip track={master} />
        </div>
      )}
    </section>
  );
}

function Strip({ track }: { track: Track }) {
  const t = useT();
  const transport = useOptionalTransport();
  const sender = useGestureSender();
  const name = useTrackDisplayName(track);
  const meter = useTrackMeter(track.id);
  const armed = useProjectStore((s) => s.armedTracks.includes(track.id));
  const db = track.mixer.volume;
  const viewer = useViewer();
  const listenBlocked = listenBlockedReason(viewer, track.id);
  const canArm = viewer === null && (track.kind === "Audio" || track.kind === "Midi");
  const send = (c: Parameters<typeof sendEdit>[1]) => transport && void sendEdit(transport, c);
  return (
    <div
      className="eth-mstrip"
      role="listitem"
      aria-label={t("mobile.mixer.channel", { name })}
      data-track={track.id}
      style={{ ["--eth-track-color" as string]: colorCss(track.color) }}
    >
      <span className="eth-mstrip__name">
        <span className="eth-mstrip__dot" aria-hidden />
        <span className="eth-mstrip__label eth-user-text">{name}</span>
      </span>
      {track.kind !== "Master" ? <HeaderPan track={track} /> : <span className="eth-mstrip__pan-space" aria-hidden />}
      <div className="eth-mstrip__fader">
        <Fader
          value={dbToFader(db)}
          label={t("mobile.mixer.volume", { name })}
          valueText={formatDb(db)}
          defaultValue={dbToFader(0)}
          onChangeStart={() => sender.begin()}
          onChangeEnd={() => sender.end()}
          onChange={(v) => {
            const next = faderToDb(v);
            if (next !== db) void sender.send(cmd("Mixer", { type: "SetVolume", track: track.id, volume: next }));
          }}
        />
        <Meter className="eth-mstrip__meter" levels={meter ? meter.peak : [0, 0]} />
      </div>
      <output className="eth-mstrip__db">{formatDb(db)}</output>
      <div className="eth-mstrip__buttons">
        <button
          type="button"
          className="eth-mstrip__toggle eth-mstrip__mute"
          aria-pressed={track.mixer.mute}
          data-solo-override={muteOverriddenBySolo(track.mixer.mute, track.mixer.solo) || undefined}
          aria-label={t("mobile.track.mute_name", { name })}
          disabled={listenBlocked !== null}
          title={listenBlocked ?? (muteOverriddenBySolo(track.mixer.mute, track.mixer.solo) ? t("mixer.mute_overridden") : undefined)}
          onClick={() => send(cmd("Mixer", { type: "SetMute", track: track.id, mute: !track.mixer.mute }))}
        >
          {track.mixer.mute ? <VolumeX aria-hidden /> : <Volume2 aria-hidden />}
        </button>
        {track.kind !== "Master" && (
          <button
            type="button"
            className="eth-mstrip__toggle eth-mstrip__solo"
            aria-pressed={track.mixer.solo}
            aria-label={t("mobile.track.solo_name", { name })}
            disabled={listenBlocked !== null}
            title={listenBlocked ?? undefined}
            onClick={() => send(trackSoloCommand(Object.values(useProjectStore.getState().project?.tracks ?? {}), track.id, false))}
          >
            <Headphones aria-hidden />
          </button>
        )}
        {canArm && (
          <button
            type="button"
            className="eth-mstrip__toggle eth-mstrip__arm"
            aria-pressed={armed}
            aria-label={t("mobile.track.arm_name", { name })}
            onClick={() =>
              transport && void transport.send(cmd("Recording", { type: "Arm", track: track.id, armed: !armed, exclusive: true })).catch(() => undefined)
            }
          >
            <Circle aria-hidden />
          </button>
        )}
      </div>
    </div>
  );
}
