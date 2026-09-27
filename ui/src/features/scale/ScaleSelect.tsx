import type { MusicalScale, ScaleKind } from "@/generated";
import { ROOT_NOTES, SCALE_KINDS, SCALES } from "@/domain/scales";
import "./scale.css";

export function ScaleSelect({ value, onChange, label, disabled = false }: {
  value: MusicalScale;
  onChange: (scale: MusicalScale) => void;
  label: string;
  disabled?: boolean;
}) {
  return <span className="eth-scale-select" role="group" aria-label={label}>
    <select aria-label={`${label} root`} title={`${label} root`} value={value.root} disabled={disabled}
      onChange={(e) => onChange({ ...value, root: Number(e.target.value) })}>
      {ROOT_NOTES.map((name, root) => <option key={name} value={root}>{name}</option>)}
    </select>
    <select aria-label={`${label} type`} title={`${label} (visual guide; notes remain unrestricted)`} value={value.kind} disabled={disabled}
      onChange={(e) => onChange({ ...value, kind: e.target.value as ScaleKind })}>
      {SCALE_KINDS.map((kind) => <option key={kind} value={kind}>{SCALES[kind].label}</option>)}
    </select>
  </span>;
}
