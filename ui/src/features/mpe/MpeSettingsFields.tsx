/**
 * MPE settings of a MIDI track in the inspector (`Expression::SetTrackMpe`, one undo step
 * per change): an on/off switch, then the zone, its member channels and the per-note and
 * master pitch-bend ranges. The inspector passes its own `Row` layout.
 */

import type { ComponentType, ReactNode } from "react";
import type { MpeSettings, MpeZone, Track } from "@/generated";
import { useT, type MessageKey } from "@/i18n";
import { NumberField, Select, Toggle } from "@/kit";
import { cmd, useTransport } from "@/transport";
import { DEFAULT_MPE, masterChannel, MAX_MEMBER_CHANNELS, MAX_PITCH_RANGE, memberChannels } from "@/domain/mpe";
import "./mpe.css";

const ZONES: ReadonlyArray<{ value: MpeZone; label: MessageKey }> = [
  { value: "Lower", label: "mpe.zone.lower" },
  { value: "Upper", label: "mpe.zone.upper" },
];

/** "2–16", or "2" for a single member channel. */
const channelRange = ([a, b]: [number, number]) => (a === b ? `${a}` : `${a}–${b}`);

export interface MpeSettingsFieldsProps {
  track: Track;
  Row: ComponentType<{ label: string; children: ReactNode }>;
}

export function MpeSettingsFields({ track, Row }: MpeSettingsFieldsProps) {
  const t = useT();
  const transport = useTransport();
  const mpe = track.mpe ?? null;
  const set = (next: MpeSettings | null) => {
    transport.send(cmd("Expression", { type: "SetTrackMpe", track: track.id, mpe: next })).catch((err: unknown) => console.warn("[mpe] command failed:", err));
  };
  const change = (patch: Partial<MpeSettings>) => mpe && set({ ...mpe, ...patch });

  return (
    <div className="eth-mpe" data-testid="mpe-settings">
      <Row label={t("mpe.enabled")}>
        <Toggle size="sm" aria-label="MPE" checked={mpe !== null} onChange={(on) => set(on ? DEFAULT_MPE : null)} />
      </Row>
      {mpe && (
        <>
          <Row label={t("mpe.zone.row")}>
            <Select
              size="sm"
              aria-label={t("mpe.zone.label")}
              value={mpe.zone}
              options={ZONES.map((z) => ({ value: z.value, label: t(z.label) }))}
              onChange={(zone) => change({ zone })}
            />
          </Row>
          <Row label={t("mpe.channels.row")}>
            <NumberField
              size="sm"
              aria-label={t("mpe.channels.label")}
              min={1}
              max={MAX_MEMBER_CHANNELS}
              value={mpe.member_channels}
              onChange={(member_channels) => change({ member_channels })}
            />
          </Row>
          <Row label={t("mpe.note_bend.row")}>
            <NumberField
              size="sm"
              aria-label={t("mpe.note_bend.label")}
              min={1}
              max={MAX_PITCH_RANGE}
              unit={t("mpe.semitones")}
              value={mpe.note_pitch_range}
              onChange={(note_pitch_range) => change({ note_pitch_range })}
            />
          </Row>
          <Row label={t("mpe.master_bend.row")}>
            <NumberField
              size="sm"
              aria-label={t("mpe.master_bend.label")}
              min={0}
              max={MAX_PITCH_RANGE}
              unit={t("mpe.semitones")}
              value={mpe.master_pitch_range}
              onChange={(master_pitch_range) => change({ master_pitch_range })}
            />
          </Row>
          <p className="eth-mpe__summary" data-testid="mpe-summary">
            {t("mpe.summary", { master: masterChannel(mpe.zone), channels: channelRange(memberChannels(mpe)) })}
          </p>
        </>
      )}
    </div>
  );
}
