/**
 * The routing panel of the External Instrument / External Audio Effect
 * (`Widget::HardwareRouting`, `external-instrument`): where the device talks to hardware.
 *
 * - Instrument: MIDI output port + channel, and the audio inputs its sound comes back on.
 * - Effect: the audio outputs its input is sent to, and the inputs it comes back on.
 * - "Measure" times the round trip and sets `Latency` (one undo step); the result or the
 *   failure shows next to it.
 * - Ports this machine doesn't have stay selected (the routing is in the document, like
 *   Ableton) and are marked missing: the device is silent until they come back.
 * - Hosts without hardware I/O (the web build) show the stored routing read-only, with a note.
 *
 * Every change is one `External::SetRouting` (one undo step).
 */

import { AudioLines, Cable, CircleAlert, Timer } from "lucide-react";
import type { ExternalRouting, WidgetSize } from "@/generated";
import { formatFixed, formatList, t as tr, useT } from "@/i18n";
import { Badge, Button, Select } from "@/kit";
import { cmd } from "@/transport";
import { useLayoutContext } from "@/features/devices/layout/context";
import { useHardwarePorts, useMeasureLatency, type MeasureState } from "./hooks";
import { midiChannels, NONE, canMeasure, channelOptions, channelsValue, externalOf, midiOptions, missingParts, parseChannels } from "./routing";
import "./external.css";

export interface HardwareRoutingWidgetProps {
  widget: { type: "HardwareRouting" };
  size: WidgetSize;
  label: string | null;
}

function measureText(m: MeasureState): string | null {
  switch (m.state) {
    case "idle":
      return null;
    case "measuring":
      return tr("external.measure.measuring");
    case "done":
      return tr("external.measure.done", { ms: formatFixed(m.latencyMs, 1) });
    case "failed":
      return m.message;
  }
}

export function HardwareRoutingWidget({ label }: HardwareRoutingWidgetProps) {
  const t = useT();
  const { device, sender } = useLayoutContext();
  const { ports, unsupported } = useHardwarePorts();
  const [measure, startMeasure] = useMeasureLatency(device.id);
  const ext = externalOf(device);
  if (!ext) return null;
  const { kind, routing } = ext;
  const instrument = kind === "ExternalInstrument";
  const set = (patch: Partial<ExternalRouting>) => {
    void sender.send(cmd("External", { type: "SetRouting", device: device.id, routing: { ...routing, ...patch } }));
  };
  const missing = missingParts(routing, ports);
  const readOnly = unsupported;
  // Without hardware I/O nothing is "missing": the stored routing is just shown.
  const missingLabel = unsupported ? null : t("external.routing.not_connected");
  const status = measureText(measure);
  return (
    <div className="eth-widget eth-widget--hardware-routing eth-hw" data-widget="HardwareRouting" data-testid="hardware-routing">
      {label && <span className="eth-widget__label">{label}</span>}
      <div className="eth-hw__rows">
        {instrument ? (
          <div className="eth-hw__row">
            <span className="eth-hw__caption">
              <Cable aria-hidden /> {t("external.routing.midi_to")}
            </span>
            <Select
              size="sm"
              aria-label={t("external.routing.midi_output")}
              disabled={readOnly}
              options={midiOptions(ports, routing.midi_out, missingLabel)}
              value={routing.midi_out ?? NONE}
              onChange={(v) => set({ midi_out: v === NONE ? null : v })}
            />
            <Select
              size="sm"
              aria-label={t("external.routing.midi_channel")}
              className="eth-hw__channel"
              disabled={readOnly}
              options={midiChannels()}
              value={`${routing.midi_channel}`}
              onChange={(v) => set({ midi_channel: Number(v) })}
            />
          </div>
        ) : (
          <div className="eth-hw__row">
            <span className="eth-hw__caption">
              <Cable aria-hidden /> {t("external.routing.audio_to")}
            </span>
            <Select
              size="sm"
              aria-label={t("external.routing.audio_send")}
              disabled={readOnly}
              options={channelOptions(ports?.audio_outputs ?? [], routing.audio_send, t("external.routing.out_prefix"), missingLabel)}
              value={channelsValue(routing.audio_send)}
              onChange={(v) => set({ audio_send: parseChannels(v) })}
            />
          </div>
        )}
        <div className="eth-hw__row">
          <span className="eth-hw__caption">
            <AudioLines aria-hidden /> {t("external.routing.audio_from")}
          </span>
          <Select
            size="sm"
            aria-label={t("external.routing.audio_return")}
            disabled={readOnly}
            options={channelOptions(ports?.audio_inputs ?? [], routing.audio_return, t("external.routing.in_prefix"), missingLabel)}
            value={channelsValue(routing.audio_return)}
            onChange={(v) => set({ audio_return: parseChannels(v) })}
          />
        </div>
        <div className="eth-hw__row eth-hw__row--measure">
          <Button
            size="sm"
            disabled={readOnly || !canMeasure(kind, routing) || measure.state === "measuring"}
            onClick={startMeasure}
            title={instrument ? t("external.measure.title_instrument") : t("external.measure.title_effect")}
          >
            <Timer aria-hidden /> {t("external.measure.button")}
          </Button>
          {status && (
            <span className="eth-hw__status" data-state={measure.state} role="status">
              {status}
            </span>
          )}
        </div>
      </div>
      {missing.length > 0 && (
        <div className="eth-hw__note eth-hw__note--warn" role="note">
          <CircleAlert aria-hidden />
          <span>
            <Badge tone="warn">{t("external.missing.badge")}</Badge> {t("external.missing.text", { parts: formatList(missing) })}
          </span>
        </div>
      )}
      {unsupported && (
        <div className="eth-hw__note" role="note">
          <CircleAlert aria-hidden />
          <span>{t("external.routing.desktop_only")}</span>
        </div>
      )}
    </div>
  );
}
