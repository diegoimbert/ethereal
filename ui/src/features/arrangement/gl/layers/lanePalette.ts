/**
 * Colors of the lane layers (automation, comping, group summaries), read from the page's
 * CSS tokens once per arranger palette (a theme switch makes a new palette).
 */

import { readToken, themeColorValue, tokenName, type ColorKey } from "@/theme";
import { mix, parseColor, type Palette, type Rgba } from "../palette";

export interface LanePalette {
  /** `--eth-color-accent`: automation envelopes and point rims. */
  accent: Rgba;
  /** `--eth-color-bg-inset`: automation lane background, point fill. */
  bgInset: Rgba;
  border: Rgba;
  borderLight: Rgba;
  selection: Rgba;
  text: Rgba;
  /** `--eth-color-text-dim`: an empty automation lane's dashed baseline. */
  textDim: Rgba;
  focus: Rgba;
}

type Key = Extract<ColorKey, "accent" | "bgInset" | "border" | "borderLight" | "selection" | "text" | "textDim" | "focus">;

const cache = new WeakMap<Palette, LanePalette>();

/** The lane colors of an arranger palette (its theme). */
export function lanePalette(p: Palette): LanePalette {
  let c = cache.get(p);
  if (c) return c;
  const color = (key: Key): Rgba => parseColor(readToken(tokenName("color", key) as `--${string}`) || themeColorValue(key));
  c = {
    accent: color("accent"),
    bgInset: color("bgInset"),
    border: color("border"),
    borderLight: color("borderLight"),
    selection: color("selection"),
    text: color("text"),
    textDim: color("textDim"),
    focus: color("focus"),
  };
  cache.set(p, c);
  return c;
}

/** `color` at `opacity` over the opaque `bg` (an element's opacity, flattened). */
export function over(color: Rgba, opacity: number, bg: Rgba): Rgba {
  return mix([color[0], color[1], color[2], 1], color[3] * opacity, bg);
}
