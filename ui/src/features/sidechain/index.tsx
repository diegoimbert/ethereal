// `SidechainSelector` is mounted in every device header by `features/devices/DeviceView`
// (next to `PluginDeviceControls`): keep its name and props.
import type { Device, SidechainTap, TrackId } from "@/generated";
import { useDescriptor } from "@/features/devices/descriptors";
import { useSend } from "@/features/devices/gesture";
import { useT } from "@/i18n";
import { Select, type SelectOption } from "@/kit";
import { useProject } from "@/state";
import { cmd } from "@/transport";
import { defaultTap, sidechainSources, TAP_ERROR_TEXT, tapError, tapKey, tapOf, tapPoints } from "./routing";

export interface SidechainSelectorProps {
  device: Device;
}

const NONE = "";

/**
 * Sidechain source selector (`Device::SetSidechain`, one undo step per change), for devices
 * at any depth (track chain, drum pads, rack chains). Renders nothing for devices without a
 * sidechain input (`DeviceDescriptor.sidechain_inputs == 0`). Like Live's "Audio From" pair:
 * the source track (every track but master; its own track for its Pre FX signal and earlier
 * racks' chains), then, once one is set, its tap point: Pre FX / Post FX / Post Mixer of the
 * track or of any of its rack chains and drum pads. Choices the engine would reject (routing
 * cycles, points produced after the device) are disabled.
 */
export function SidechainSelector({ device }: SidechainSelectorProps) {
  const t = useT();
  const { descriptor } = useDescriptor(device);
  const project = useProject();
  const send = useSend();
  if (!descriptor || descriptor.sidechain_inputs === 0 || !project) return null;

  const set = (source: TrackId | null, tap: SidechainTap | null) =>
    void send(
      cmd("Device", {
        type: "SetSidechain",
        device: device.id,
        source,
        tap: tap ?? undefined,
      }),
    );

  const options: SelectOption<string>[] = [
    { value: NONE, label: t("sidechain.none") },
    ...sidechainSources(project, device).map(({ track, cycle }) => {
      const own = track.id === device.track;
      const usable = own ? defaultTap(project, device, track.id) !== null : !cycle;
      return {
        value: track.id,
        label: cycle ? t("sidechain.source_loop", { name: track.name }) : own ? t("sidechain.source_own", { name: track.name }) : track.name,
        userText: !cycle && !own,
        disabled: !usable && track.id !== device.sidechain,
      };
    }),
  ];
  const source = device.sidechain;
  const current = tapOf(device);
  const points: SelectOption<string>[] =
    source === null
      ? []
      : tapPoints(project, source).map((o) => {
          const why = tapKey(o.tap) === tapKey(current) ? null : tapError(project, device, source, o.tap);
          return {
            value: o.key,
            label: why ? t("sidechain.tap_refused", { tap: o.label, reason: t(TAP_ERROR_TEXT[why]) }) : o.label,
            disabled: why !== null,
          };
        });
  const byKey = new Map(source === null ? [] : tapPoints(project, source).map((o) => [o.key, o.tap]));
  return (
    <>
      <Select
        size="sm"
        className="eth-sidechain"
        aria-label={t("sidechain.source_label", { name: device.name })}
        title={t("sidechain.source_title")}
        options={options}
        value={source ?? NONE}
        onChange={(v) => {
          if (v === NONE) return set(null, null);
          set(v, defaultTap(project, device, v));
        }}
      />
      {source !== null && (
        <Select
          size="sm"
          className="eth-sidechain-tap"
          aria-label={t("sidechain.tap_label", { name: device.name })}
          title={t("sidechain.tap_title")}
          options={points}
          value={tapKey(current)}
          onChange={(k) => {
            const tap = byKey.get(k);
            if (tap) set(source, tap);
          }}
        />
      )}
    </>
  );
}
