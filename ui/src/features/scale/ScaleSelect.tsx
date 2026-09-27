import type { MusicalScale, ScaleKind } from "@/generated";
import { Select } from "@/kit";
import { ROOT_NOTES, SCALE_KINDS, SCALES } from "@/domain/scales";
import "./scale.css";

const ROOT_OPTIONS = ROOT_NOTES.map((name, root) => ({ value: String(root), label: name }));
const KIND_OPTIONS = SCALE_KINDS.map((kind) => ({ value: kind, label: SCALES[kind].label }));

/** Root + scale-type pickers (kit Selects) for a `MusicalScale`. */
export function ScaleSelect({
  value,
  onChange,
  label,
  disabled = false,
}: {
  value: MusicalScale;
  onChange: (scale: MusicalScale) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <span className="eth-scale-select" role="group" aria-label={label}>
      <Select
        size="sm"
        aria-label={`${label} root`}
        title={`${label} root`}
        value={String(value.root)}
        options={ROOT_OPTIONS}
        disabled={disabled}
        onChange={(root) => onChange({ ...value, root: Number(root) })}
      />
      <Select<ScaleKind>
        size="sm"
        aria-label={`${label} type`}
        title={`${label} (visual guide; notes remain unrestricted)`}
        value={value.kind}
        options={KIND_OPTIONS}
        disabled={disabled}
        onChange={(kind) => onChange({ ...value, kind })}
      />
    </span>
  );
}
