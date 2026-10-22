// Colors of the clip-editing layers, read from the theme tokens `clipEditing.css` and
// `mediaRefs.css` use (cached per palette: a theme switch reads a new palette).
import { readToken, themeColorValue, tokenName, type ColorKey } from "@/theme";
import { parseColor, withAlpha, type Palette, type Rgba } from "../palette";

export interface EditColors {
  /** Fade shade (`--eth-color-backdrop`). */
  shade: Rgba;
  /** Fade curve and handle fill (`--eth-color-text`). */
  line: Rgba;
  /** The crossfade neighbour's curve (`--eth-color-text-dim`). */
  neighbour: Rgba;
  /** Crossfade region (`color-mix(selection 22%, transparent)`). */
  tint: Rgba;
  /** Handle border (`--eth-color-bg`) and hovered handle (`--eth-color-selection`). */
  handleBorder: Rgba;
  handleHover: Rgba;
  /** Badges: "REV" (`accent`), "Missing" (`warn`), their text (`text-inverse`). */
  accent: Rgba;
  warn: Rgba;
  badgeText: Rgba;
  /** Missing media: the clip's hatch (`color-mix(warn 22%, transparent)`). */
  hatch: Rgba;
  /** An extension's badge (`text-dim`) and its soft hatch. */
  dim: Rgba;
  softHatch: Rgba;
}

const cache = new WeakMap<Palette, EditColors>();

function token(key: ColorKey): Rgba {
  const css = readToken(tokenName("color", key) as `--${string}`) || themeColorValue(key);
  return parseColor(css);
}

export function editColors(p: Palette): EditColors {
  let c = cache.get(p);
  if (c) return c;
  const warn = token("warn");
  const dim = token("textDim");
  c = {
    shade: token("backdrop"),
    line: token("text"),
    neighbour: token("textDim"),
    tint: withAlpha(p.selection, 0.22),
    handleBorder: p.bg,
    handleHover: p.selection,
    accent: token("accent"),
    warn,
    badgeText: p.titleText,
    hatch: withAlpha(warn, 0.22),
    dim,
    softHatch: withAlpha(dim, 0.16),
  };
  cache.set(p, c);
  return c;
}
