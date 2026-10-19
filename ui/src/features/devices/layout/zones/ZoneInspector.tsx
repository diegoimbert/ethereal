/**
 * Fields of the selected multisampler zone: ranges, root/tune/gain/pan, round-robin group,
 * playback range and sustain loop. Every change is one `Device::SetZones` (a field drag is
 * one undo gesture through the panel's gesture sender).
 */

import type { ReactNode } from "react";
import type { SampleZone } from "@/generated";
import { Button, NumberField, Toggle } from "@/kit";
import { useProjectStore } from "@/state";
import { useLayoutContext } from "../context";
import { KEYS, VELS, noteName } from "./zoneMath";
import { useT } from "@/i18n";

export interface ZoneInspectorProps {
  zone: SampleZone;
  index: number;
  onChange(zone: SampleZone): void;
  onRemove(): void;
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="eth-zones__field">
      <span className="eth-zones__field-label" title={label}>
        {label}
      </span>
      {children}
    </label>
  );
}

export function ZoneInspector({ zone: z, index, onChange, onRemove }: ZoneInspectorProps) {
  const t = useT();
  const { sender } = useLayoutContext();
  const media = useProjectStore((s) => (z.media ? s.project?.media[z.media] : undefined));
  const duration = media && media.sample_rate > 0 ? media.frames / media.sample_rate : 0;
  const end = z.end ?? duration;
  const set = (patch: Partial<SampleZone>) => onChange({ ...z, ...patch });
  const drag = { onChangeStart: sender.begin, onChangeEnd: sender.end, size: "sm" as const };
  const num = (label: string, value: number, apply: (v: number) => void, o: { min: number; max: number; step?: number; precision?: number; unit?: string }) => (
    <Field label={label}>
      <NumberField aria-label={label} value={value} onChange={apply} {...o} {...drag} />
    </Field>
  );
  const seconds = { min: 0, max: Math.max(duration, end, z.loop_end, 0.001), step: 0.001, precision: 3, unit: "s" };

  return (
    <div className="eth-zones__inspector" data-testid="zone-inspector" aria-label={t("devices.zones.zone", { n: index + 1 })} role="group">
      <div className="eth-zones__inspector-head">
        <span className="eth-zones__sample" title={media?.name}>
          {t("devices.zones.zone", { n: index + 1 })}
          {" · "}
          {z.media ? media ? <span className="eth-user-text">{media.name}</span> : t("devices.zones.missing_sample") : t("devices.zones.empty")}
        </span>
        <Button size="sm" tone="ghost" aria-label={t("devices.zones.remove")} title={t("devices.zones.remove_hint")} onClick={onRemove}>
          ✕
        </Button>
      </div>
      <div className="eth-zones__fields">
        {num(t("devices.zones.key_low"), z.keys.lo, (v) => set({ keys: { lo: v, hi: Math.max(v, z.keys.hi) } }), { min: 0, max: KEYS - 1 })}
        {num(t("devices.zones.key_high"), z.keys.hi, (v) => set({ keys: { lo: Math.min(v, z.keys.lo), hi: v } }), { min: 0, max: KEYS - 1 })}
        {num(t("devices.zones.vel_low"), Math.max(1, z.velocities.lo), (v) => set({ velocities: { lo: v, hi: Math.max(v, z.velocities.hi) } }), {
          min: 1,
          max: VELS,
        })}
        {num(t("devices.zones.vel_high"), z.velocities.hi, (v) => set({ velocities: { lo: Math.min(v, Math.max(1, z.velocities.lo)), hi: v } }), {
          min: 1,
          max: VELS,
        })}
        {num(t("devices.zones.root"), z.root_key, (v) => set({ root_key: v }), { min: 0, max: KEYS - 1, unit: noteName(z.root_key) })}
        {num(t("devices.zones.tune"), z.tune_cents, (v) => set({ tune_cents: v }), { min: -100, max: 100, unit: "ct" })}
        {num(t("devices.zones.gain"), z.gain, (v) => set({ gain: v }), { min: -70, max: 24, step: 0.1, precision: 1, unit: "db" })}
        {num(t("devices.zones.pan"), z.pan, (v) => set({ pan: v }), { min: -1, max: 1, step: 0.01, precision: 2 })}
        {num(t("devices.zones.round_robin"), z.round_robin, (v) => set({ round_robin: v }), { min: 0, max: 255 })}
        {num(t("devices.zones.start"), z.start, (v) => set({ start: v }), seconds)}
        {num(t("devices.zones.end"), end, (v) => set({ end: duration > 0 && Math.abs(v - duration) < 1e-6 ? null : v }), seconds)}
        <Field label={t("devices.zones.loop")}>
          <Toggle
            size="sm"
            aria-label={t("devices.zones.loop")}
            checked={z.looping}
            onChange={(on) => set(on && z.loop_end <= z.loop_start ? { looping: true, loop_start: z.start, loop_end: end } : { looping: on })}
          />
        </Field>
        {z.looping && (
          <>
            {num(t("devices.zones.loop_start"), z.loop_start, (v) => set({ loop_start: v }), seconds)}
            {num(t("devices.zones.loop_end"), z.loop_end, (v) => set({ loop_end: v }), seconds)}
            {num(t("devices.zones.crossfade"), z.loop_crossfade, (v) => set({ loop_crossfade: v }), {
              ...seconds,
              max: Math.max(0.001, z.loop_end - z.loop_start),
            })}
          </>
        )}
      </div>
    </div>
  );
}
