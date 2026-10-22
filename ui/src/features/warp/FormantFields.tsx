/**
 * Complex+'s formant settings of one or several audio clips (Live's Complex Pro "Formants"
 * and "Envelope", plus a formant shift): `formants` (0–100 %: how much of the voice's
 * character stays in place when transposing), `formant shift` (semitones) and `envelope`
 * (detail of the spectral envelope, 8–256). Shown only for Complex+ clips, by the warp
 * editor's toolbar and the inspector.
 *
 * Several clips edit together like the transpose (`multiEdit`): a drag or an arrow step moves
 * each clip from its own value, a typed value sets them all, differing values show as mixed.
 */

import { useRef, type ReactNode } from "react";
import { NumberField } from "@/kit";
import {
  common,
  FORMANT_RANGE,
  formantBases,
  formantsOf,
  offsetFormant,
  setAllFormant,
  type AudioClip,
  type Bases,
  type FormantKey,
} from "@/features/clip-editing/multiEdit";
import { useTransport } from "@/transport";
import { useT, type MessageKey } from "@/i18n";
import { openGesture, sendOne } from "./drag";

interface Spec {
  key: FormantKey;
  label: MessageKey;
  title: MessageKey;
  /** Shown value = stored value × scale (the amount is stored 0–1, shown in %). */
  scale: number;
  step: number;
  precision: number;
  unit: MessageKey | null;
}

const SPECS: ReadonlyArray<Spec> = [
  { key: "amount", label: "warp.formants", title: "warp.formants_title", scale: 100, step: 1, precision: 0, unit: "warp.percent_unit" },
  { key: "shift", label: "warp.formant_shift", title: "warp.formant_shift_title", scale: 1, step: 1, precision: 1, unit: "warp.semitones_unit" },
  { key: "envelope", label: "warp.envelope", title: "warp.envelope_title", scale: 1, step: 1, precision: 0, unit: null },
];

/**
 * The three fields for `targets`, each laid out by `row(key, label, title, field)`.
 */
export function FormantFields({
  targets,
  row,
}: {
  targets: ReadonlyArray<AudioClip>;
  row: (key: string, label: string, title: string, field: ReactNode) => ReactNode;
}) {
  const t = useT();
  const transport = useTransport();
  const gesture = useRef<{ g: ReturnType<typeof openGesture>; bases: Bases } | null>(null);
  if (targets.length === 0) return null;
  const first = formantsOf(targets[0]!);
  return (
    <>
      {SPECS.map((s) => {
        const value = common(targets.map((c) => formantsOf(c)[s.key]));
        const [lo, hi] = FORMANT_RANGE[s.key];
        const send = (c: ReturnType<typeof setAllFormant>) => {
          if (!c) return;
          if (gesture.current) gesture.current.g.send(c);
          else sendOne(transport, c);
        };
        const field = (
          <NumberField
            size="sm"
            aria-label={t(s.label)}
            className="eth-warp__number"
            data-testid={`warp-formant-${s.key}`}
            value={(value ?? first[s.key]) * s.scale}
            mixed={value === null}
            min={lo * s.scale}
            max={hi * s.scale}
            step={s.step}
            precision={s.precision}
            unit={s.unit ? t(s.unit) : undefined}
            onChangeStart={() => {
              gesture.current = { g: openGesture(transport), bases: formantBases(targets, s.key) };
            }}
            onChangeEnd={() => {
              gesture.current?.g.end();
              gesture.current = null;
            }}
            // A typed value: every target gets it; a drag or a step moves each from its own.
            onChange={(v) => send(setAllFormant(targets, s.key, v / s.scale))}
            onNudge={(d) => send(offsetFormant(targets, s.key, gesture.current?.bases ?? formantBases(targets, s.key), d / s.scale))}
          />
        );
        return row(s.key, t(s.label), t(s.title), field);
      })}
    </>
  );
}
