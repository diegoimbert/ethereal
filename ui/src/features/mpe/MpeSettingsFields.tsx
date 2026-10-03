/**
 * MPE settings of a MIDI track in the inspector (`Expression::SetTrackMpe`, one undo step
 * per change): an on/off switch, then the zone, its member channels and the per-note and
 * master pitch-bend ranges. The inspector passes its own `Row` layout.
 */

import type { ComponentType, ReactNode } from "react";
import type { MpeSettings, MpeZone, Track } from "@/generated";
import { NumberField, Select, Toggle, type SelectOption } from "@/kit";
import { cmd, useTransport } from "@/transport";
import { DEFAULT_MPE, MAX_MEMBER_CHANNELS, MAX_PITCH_RANGE, zoneSummary } from "./model";
import "./mpe.css";

const ZONES: ReadonlyArray<SelectOption<MpeZone>> = [
  { value: "Lower", label: "Lower (master 1)" },
  { value: "Upper", label: "Upper (master 16)" },
];

export interface MpeSettingsFieldsProps {
  track: Track;
  Row: ComponentType<{ label: string; children: ReactNode }>;
}

export function MpeSettingsFields({ track, Row }: MpeSettingsFieldsProps) {
  const transport = useTransport();
  const mpe = track.mpe ?? null;
  const set = (next: MpeSettings | null) => {
    transport
      .send(cmd("Expression", { type: "SetTrackMpe", track: track.id, mpe: next }))
      .catch((err: unknown) => console.warn("[mpe] command failed:", err));
  };
  const change = (patch: Partial<MpeSettings>) => mpe && set({ ...mpe, ...patch });

  return (
    <div className="eth-mpe" data-testid="mpe-settings">
      <Row label="MPE">
        <Toggle size="sm" aria-label="MPE" checked={mpe !== null} onChange={(on) => set(on ? DEFAULT_MPE : null)} />
      </Row>
      {mpe && (
        <>
          <Row label="Zone">
            <Select size="sm" aria-label="MPE zone" value={mpe.zone} options={ZONES} onChange={(zone) => change({ zone })} />
          </Row>
          <Row label="Channels">
            <NumberField
              size="sm"
              aria-label="MPE member channels"
              min={1}
              max={MAX_MEMBER_CHANNELS}
              value={mpe.member_channels}
              onChange={(member_channels) => change({ member_channels })}
            />
          </Row>
          <Row label="Note bend">
            <NumberField
              size="sm"
              aria-label="Per-note pitch bend range"
              min={1}
              max={MAX_PITCH_RANGE}
              unit="st"
              value={mpe.note_pitch_range}
              onChange={(note_pitch_range) => change({ note_pitch_range })}
            />
          </Row>
          <Row label="Master bend">
            <NumberField
              size="sm"
              aria-label="Master pitch bend range"
              min={0}
              max={MAX_PITCH_RANGE}
              unit="st"
              value={mpe.master_pitch_range}
              onChange={(master_pitch_range) => change({ master_pitch_range })}
            />
          </Row>
          <p className="eth-mpe__summary" data-testid="mpe-summary">
            {zoneSummary(mpe)}
          </p>
        </>
      )}
    </div>
  );
}
