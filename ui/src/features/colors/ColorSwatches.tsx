import { displayColorCss, TRACK_COLORS } from "@/theme";
import { useT } from "@/i18n";
import "./colors.css";

/** The palette's colors as 0xRRGGBB. */
export const PALETTE: ReadonlyArray<number> = TRACK_COLORS.map((c) => parseInt(c.slice(1), 16));

/** A track / clip color as CSS, as shown (`displayColor`); an unknown one (`null`) is the fallback token. */
export const colorSwatchCss = (color: number | null | undefined): string => (color == null ? "var(--eth-track-fallback)" : displayColorCss(color));

/**
 * The palette's color swatches (a radio group); `inherit` adds a "follow the track color"
 * swatch (value null), showing that color (`null`: unknown, several tracks).
 */
export function ColorSwatches({
  value,
  onPick,
  inherit,
  label,
}: {
  value: number | null;
  onPick(color: number | null): void;
  inherit?: number | null;
  label: string;
}) {
  const t = useT();
  return (
    <div className="eth-color-swatches" role="radiogroup" aria-label={label}>
      {inherit !== undefined && (
        <button
          type="button"
          role="radio"
          aria-checked={value === null}
          aria-label={t("shell.inspector.track_color")}
          title={t("shell.inspector.use_track_color")}
          className="eth-color-swatch eth-color-swatch--inherit"
          style={{ ["--swatch" as string]: colorSwatchCss(inherit) }}
          onClick={() => onPick(null)}
        />
      )}
      {PALETTE.map((c) => (
        <button
          key={c}
          type="button"
          role="radio"
          aria-checked={value === c}
          aria-label={colorSwatchCss(c)}
          className="eth-color-swatch"
          style={{ ["--swatch" as string]: colorSwatchCss(c) }}
          onClick={() => onPick(c)}
        />
      ))}
    </div>
  );
}
