/**
 * The Track sheet (the design's §4 "Track headers"): everything a track header does on
 * desktop, at touch size, opened by tapping a selected track's header. Mute / solo / arm,
 * volume and pan (the header's own fader and knob, larger), rename, move up / down, and the
 * track menu's actions (cut, copy, paste, duplicate, delete). The device chain is listed
 * read-only (name and on / off): devices are not adapted to touch yet.
 */
import { useEffect } from "react";
import { ArrowDown, ArrowUp, Circle, Headphones, Spline, Volume2, VolumeX } from "lucide-react";
import { AutomationToggleButton } from "@/features/automation";
import { useDeviceOn } from "@/features/automation/live";
import type { Command, Device, Track } from "@/generated";
import { Button, type ContextMenuItem } from "@/kit";
import { useDevicesOfTrack, useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { trackSoloCommand } from "@/domain/soloCommands";
import { trackMenu } from "@/features/arrangement/actions";
import { sendEdit } from "@/features/arrangement/context";
import { HeaderPan } from "@/features/arrangement/HeaderPan";
import { HeaderVolume } from "@/features/arrangement/HeaderVolume";
import { colorCss } from "@/features/arrangement/helpers";
import { useTrackDisplayName } from "@/features/arrangement/trackDisplayName";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { siblingMove } from "./trackMoves";
import { closeSheet, useMobileSheets } from "./sheets";
import { Sheet } from "./Sheet";
import { useT } from "@/i18n";

export function TrackSheetRoot() {
  const t = useT();
  const sheet = useMobileSheets((s) => s.sheet);
  const track = useProjectStore((s) => (sheet?.kind === "track" ? (s.project?.tracks[sheet.track] ?? null) : null));
  // The track went away (deleted, undone): the sheet closes.
  useEffect(() => {
    if (sheet?.kind === "track" && !track) closeSheet();
  }, [sheet, track]);
  return (
    <Sheet open={!!track} onClose={closeSheet} label={t("mobile.track.sheet")} title={track ? <TrackTitle track={track} /> : null} testId="track-sheet">
      {track && <TrackSheetBody track={track} />}
    </Sheet>
  );
}

function TrackTitle({ track }: { track: Track }) {
  const name = useTrackDisplayName(track);
  return (
    <span className="eth-track-sheet__title" style={{ ["--eth-track-color" as string]: colorCss(track.color) }}>
      <span className="eth-track-sheet__swatch" aria-hidden />
      <span className="eth-user-text">{name}</span>
    </span>
  );
}

/** A device and whether it runs now (its "Device On" automation at the playhead, else its switch). */
function DeviceRow({ device }: { device: Device }) {
  const t = useT();
  const { on } = useDeviceOn(device);
  return (
    <li className="eth-track-sheet__device" data-enabled={on}>
      <span className="eth-track-sheet__device-dot" aria-hidden />
      <span className="eth-user-text">{device.name}</span>
      <span className="eth-track-sheet__device-state">{on ? t("mobile.track.device_on") : t("common.off")}</span>
    </li>
  );
}

function TrackSheetBody({ track }: { track: Track }) {
  const t = useT();
  const transport = useOptionalTransport();
  const devices = useDevicesOfTrack(track.id);
  const armed = useProjectStore((s) => s.armedTracks.includes(track.id));
  if (!transport) return null;
  const send = (c: Command) => void sendEdit(transport, c);
  const mute = track.mixer.mute;
  const solo = track.mixer.solo;
  const canArm = track.kind === "Audio" || track.kind === "Midi";
  const project = useProjectStore.getState().project;
  const up = project ? siblingMove(project, track.id, -1) : null;
  const down = project ? siblingMove(project, track.id, 1) : null;
  const actions = trackMenu(transport, track).filter((e): e is ContextMenuItem => e !== "separator");
  const rename = (field: HTMLInputElement) => {
    const next = field.value.trim();
    if (next && next !== track.name) send(cmd("Track", { type: "Rename", id: track.id, name: next }));
    else field.value = track.name;
  };

  return (
    <div className="eth-track-sheet" style={{ ["--eth-track-color" as string]: colorCss(track.color) }}>
      <label className="eth-track-sheet__row">
        <span className="eth-track-sheet__label">{t("mobile.track.name_label")}</span>
        <input
          // A rename elsewhere (undo, a peer) shows here.
          key={track.name}
          className="eth-track-sheet__name"
          aria-label={t("mobile.track.name")}
          defaultValue={track.name}
          onBlur={(e) => rename(e.currentTarget)}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
        />
      </label>
      <div className="eth-track-sheet__toggles" role="group" aria-label={t("mobile.track.switches")}>
        <Button
          className="eth-track-sheet__toggle"
          active={mute}
          aria-label={t("mobile.track.mute_name", { name: track.name })}
          onClick={() => send(cmd("Mixer", { type: "SetMute", track: track.id, mute: !mute }))}
        >
          {mute ? <VolumeX aria-hidden /> : <Volume2 aria-hidden />} {t("mobile.track.mute")}
        </Button>
        {track.kind !== "Master" && (
          <Button
            className="eth-track-sheet__toggle"
            active={solo}
            aria-label={t("mobile.track.solo_name", { name: track.name })}
            onClick={() => send(trackSoloCommand(Object.values(useProjectStore.getState().project?.tracks ?? {}), track.id, false))}
          >
            <Headphones aria-hidden /> {t("mobile.track.solo")}
          </Button>
        )}
        {canArm && (
          <Button
            className="eth-track-sheet__toggle eth-track-sheet__arm"
            active={armed}
            aria-label={t("mobile.track.arm_name", { name: track.name })}
            onClick={() => void transport.send(cmd("Recording", { type: "Arm", track: track.id, armed: !armed, exclusive: true })).catch(() => undefined)}
          >
            <Circle aria-hidden /> {t("mobile.track.arm")}
          </Button>
        )}
        {/* The track's automation lanes, under it in the arrangement (view and edit by touch). */}
        <AutomationToggleButton trackId={track.id} trackName={track.name} className="eth-button eth-button--default eth-button--md eth-track-sheet__toggle">
          <Spline aria-hidden /> {t("mobile.track.automation")}
        </AutomationToggleButton>
      </div>
      <div className="eth-track-sheet__row">
        <span className="eth-track-sheet__label">{t("mobile.track.volume")}</span>
        <span className="eth-track-sheet__fader">
          <HeaderVolume track={track} />
        </span>
      </div>
      {track.kind !== "Master" && (
        <div className="eth-track-sheet__row">
          <span className="eth-track-sheet__label">{t("mobile.track.pan")}</span>
          <HeaderPan track={track} />
        </div>
      )}
      {track.kind !== "Master" && (
        <div className="eth-track-sheet__actions" role="group" aria-label={t("mobile.track.actions")}>
          <Button className="eth-track-sheet__action" disabled={!up} onClick={() => up && send(up)}>
            <ArrowUp aria-hidden /> {t("mobile.track.move_up")}
          </Button>
          <Button className="eth-track-sheet__action" disabled={!down} onClick={() => down && send(down)}>
            <ArrowDown aria-hidden /> {t("mobile.track.move_down")}
          </Button>
          {actions.map((a) => (
            <Button
              key={a.label}
              className="eth-track-sheet__action"
              tone={a.danger ? "danger" : undefined}
              disabled={a.disabled}
              onClick={() => {
                a.onSelect();
                if (a.danger) closeSheet();
              }}
            >
              {a.label}
            </Button>
          ))}
        </div>
      )}
      <section className="eth-track-sheet__devices" aria-label={t("mobile.track.devices")}>
        <h3 className="eth-track-sheet__heading">{t("mobile.track.devices")}</h3>
        {devices.length === 0 ? (
          <p className="eth-track-sheet__empty">{t("mobile.track.no_devices")}</p>
        ) : (
          <ul className="eth-track-sheet__device-list">
            {devices.map((d) => (
              <DeviceRow key={d.id} device={d} />
            ))}
          </ul>
        )}
        <p className="eth-track-sheet__hint">{t("mobile.track.devices_hint")}</p>
      </section>
    </div>
  );
}
