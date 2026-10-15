/**
 * Text pre-rendered once per (text, font, color) for canvases that redraw often (ruler labels,
 * clip names on the small-clips canvases). Setting a canvas font and `fillText` are costly on
 * every call (font matching, shaping through the fallback list); drawing a cached sprite is a
 * plain image copy.
 */

export interface TextSprite {
  canvas: HTMLCanvasElement;
  /** CSS px. */
  width: number;
  height: number;
}

const caches = new Map<string, Map<string, TextSprite>>();
let measurer: CanvasRenderingContext2D | null | undefined;
let measuredFont = "";

/** A canvas holding `text` in `font`/`color`, `height` CSS px tall (text vertically centered). */
export function textSprite(text: string, font: string, color: string, height: number, dpr: number): TextSprite {
  const key = `${font}|${color}|${height}|${dpr}`;
  let cache = caches.get(key);
  if (!cache) caches.set(key, (cache = new Map()));
  let sprite = cache.get(text);
  if (sprite) return sprite;
  if (measurer === undefined) measurer = document.createElement("canvas").getContext("2d");
  let width = text.length * 7;
  if (measurer) {
    if (measuredFont !== font) {
      measurer.font = font;
      measuredFont = font;
    }
    const m = measurer.measureText?.(text);
    if (m) width = Math.ceil(m.width) + 1;
  }
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(width * dpr));
  canvas.height = Math.max(1, Math.round(height * dpr));
  const ctx = canvas.getContext("2d");
  if (ctx) {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.font = font;
    ctx.textBaseline = "middle";
    ctx.fillStyle = color;
    ctx.fillText(text, 0, height / 2);
  }
  sprite = { canvas, width, height };
  if (cache.size > 4000) cache.clear();
  cache.set(text, sprite);
  return sprite;
}
