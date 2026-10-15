import type { MusicalScale, ScaleKind } from "@/generated";
import { Select } from "@/kit";
import { ROOT_NOTES, SCALE_KINDS, scaleKindName } from "@/domain/scales";
import { useT } from "@/i18n";
import "./scale.css";

const ROOT_OPTIONS = ROOT_NOTES.map((name, root) => ({ value: String(root), label: name, userText: true }));

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
  const t = useT(); // the scale names follow a language switch too
  const kindOptions = SCALE_KINDS.map((kind) => ({ value: kind, label: scaleKindName(kind) }));
  return (
    <span className="eth-scale-select" role="group" aria-label={label}>
      <Select
        size="sm"
        aria-label={t("scale.select.root", { label })}
        title={t("scale.select.root", { label })}
        value={String(value.root)}
        options={ROOT_OPTIONS}
        disabled={disabled}
        onChange={(root) => onChange({ ...value, root: Number(root) })}
      />
      <Select<ScaleKind>
        size="sm"
        aria-label={t("scale.select.type", { label })}
        title={t("scale.select.type_title", { label })}
        value={value.kind}
        options={kindOptions}
        disabled={disabled}
        onChange={(kind) => onChange({ ...value, kind })}
      />
    </span>
  );
}
