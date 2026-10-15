/**
 * The `shape` pipeline of the lane layers (a layer's own pipeline, see `LayerOutput.custom`):
 * what the core `rect` pipeline can't draw.
 *
 * - **segment** (`writeSegment`): an SDF capsule from A to B: polyline pieces (automation
 *   envelopes; round joins come for free) and, with A = B, filled and stroked circles
 *   (automation breakpoints).
 * - **outline** (`writeOutline`): a rounded rectangle's inner stroke, solid or dashed (the
 *   comp's piece outlines, CSS `outline: dashed`).
 *
 * Positions are in lane space like the core pipelines (shaders.ts): x is a beat plus px, y
 * is px from the row top (`u_row.x`). Each instance also carries a vertical clip range (px
 * from the row top), e.g. the automation lane it belongs to.
 */

import { Pipeline } from "../glContext";
import type { Rgba } from "../palette";
import type { InstanceWriter } from "../pipelines";

type InstanceWriterLike = InstanceWriter;

const v4 = (name: string) => ({ name, size: 4 });

export const SHAPE_VS = /* glsl */ `#version 300 es
precision highp float;
uniform vec4 u_view;   // scroll beats (hi, lo), px per beat, x of the lane origin (css px)
uniform vec4 u_canvas; // canvas width, height (css px), device pixel ratio, content y at the canvas top
uniform vec2 u_row;    // row top (content px), main lane height (px)
in vec2 a_corner;
in vec4 i_x;      // beat, px of A; beat, px of B
in vec4 i_y;      // y of A, y of B (px from the row top), radius, inner stroke width
in vec4 i_fill;
in vec4 i_stroke;
in vec4 i_m;      // mode (0 segment, 1 outline), dash length (px, 0 = solid), min width, alpha
in vec4 i_c;      // clip y0, y1 (px from the row top), unused, unused
out vec2 v_pos;
flat out vec4 v_seg;
flat out vec4 v_fill;
flat out vec4 v_stroke;
flat out vec4 v_p;   // radius, stroke width, mode, dash
flat out vec3 v_clip; // clip y0, y1 (canvas px), alpha

float bx(float b) { return ((b - u_view.x) - u_view.y) * u_view.z + u_view.w; }
float rowY(float a) { return u_row.x - u_canvas.w + a; }

void main() {
  vec2 a = vec2(bx(i_x.x) + i_x.y, rowY(i_y.x));
  vec2 b = vec2(bx(i_x.z) + i_x.w, rowY(i_y.y));
  float r = i_y.z;
  bool outline = i_m.x > 0.5;
  vec2 lo;
  vec2 hi;
  if (outline) {
    b.x = max(b.x, a.x + i_m.z);
    lo = min(a, b) - 1.0;
    hi = max(a, b) + 1.0;
  } else {
    // Clip long segments to the canvas (with room for the caps): the SDF stays precise for
    // the lines that run to the lane's ends.
    float pad = r + 2.0;
    vec2 d = b - a;
    if (abs(d.x) > 1e-3) {
      float t0 = (-pad - a.x) / d.x;
      float t1 = (u_canvas.x + pad - a.x) / d.x;
      float ta = clamp(min(t0, t1), 0.0, 1.0);
      float tb = clamp(max(t0, t1), 0.0, 1.0);
      vec2 a2 = a + d * ta;
      b = a + d * tb;
      a = a2;
    }
    lo = min(a, b) - pad;
    hi = max(a, b) + pad;
  }
  float c0 = rowY(i_c.x);
  float c1 = rowY(i_c.y);
  lo = vec2(max(lo.x, -4.0), max(lo.y, c0 - 1.0));
  hi = vec2(min(hi.x, u_canvas.x + 4.0), min(hi.y, c1 + 1.0));
  v_pos = mix(lo, hi, a_corner);
  gl_Position = hi.x > lo.x && hi.y > lo.y
    ? vec4(v_pos.x / u_canvas.x * 2.0 - 1.0, 1.0 - v_pos.y / u_canvas.y * 2.0, 0.0, 1.0)
    : vec4(2.0, 2.0, 2.0, 1.0);
  v_seg = vec4(a, b);
  v_fill = i_fill;
  v_stroke = i_stroke;
  v_p = vec4(r, i_y.w, i_m.x, i_m.y);
  v_clip = vec3(c0, c1, i_m.w);
}
`;

export const SHAPE_FS = /* glsl */ `#version 300 es
precision highp float;
uniform vec4 u_canvas;
in vec2 v_pos;
flat in vec4 v_seg;
flat in vec4 v_fill;
flat in vec4 v_stroke;
flat in vec4 v_p;
flat in vec3 v_clip;
out vec4 o_color;

float coverage(float d) { return clamp(0.5 - d * u_canvas.z, 0.0, 1.0); }
float sdBox(vec2 p, vec4 r, float rad) {
  vec2 c = (r.xy + r.zw) * 0.5;
  vec2 h = max((r.zw - r.xy) * 0.5, vec2(0.0));
  rad = min(rad, min(h.x, h.y));
  vec2 q = abs(p - c) - h + rad;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rad;
}
// 1 on a dash, 0 in a gap: position along the rectangle's perimeter (clockwise from the
// top-left corner, by the nearest edge).
float dashOn(vec2 p, vec4 r, float dash) {
  float w = r.z - r.x;
  float h = r.w - r.y;
  float dl = p.x - r.x;
  float dr = r.z - p.x;
  float dt = p.y - r.y;
  float db = r.w - p.y;
  float m = min(min(dl, dr), min(dt, db));
  float s = m == dt ? dl : m == dr ? w + dt : m == db ? w + h + dr : 2.0 * w + h + db;
  return mod(s, 2.0 * dash) < dash ? 1.0 : 0.0;
}

void main() {
  float clipCov = clamp(min(v_pos.y - v_clip.x, v_clip.y - v_pos.y) * u_canvas.z + 0.5, 0.0, 1.0);
  if (clipCov <= 0.0) discard;
  float d;
  vec4 rect = vec4(min(v_seg.xy, v_seg.zw), max(v_seg.xy, v_seg.zw));
  if (v_p.z > 0.5) {
    d = sdBox(v_pos, rect, v_p.x);
  } else {
    vec2 pa = v_pos - v_seg.xy;
    vec2 ba = v_seg.zw - v_seg.xy;
    float t = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    d = length(pa - ba * t) - v_p.x;
  }
  float c = coverage(d);
  vec4 o = vec4(v_fill.rgb * v_fill.a, v_fill.a) * c;
  if (v_p.y > 0.0) {
    float sc = max(0.0, c - coverage(d + v_p.y));
    if (v_p.z > 0.5 && v_p.w > 0.0) sc *= dashOn(v_pos, rect, v_p.w);
    o = vec4(v_stroke.rgb * v_stroke.a, v_stroke.a) * sc + o * (1.0 - v_stroke.a * sc);
  }
  o_color = o * clipCov * v_clip.z;
  if (o_color.a <= 0.0) discard;
}
`;

export const SHAPE_STRIDE = 24;

/** A new `shape` pipeline (one per renderer: programs belong to its context). */
export const shapePipeline = () => new Pipeline("shape", SHAPE_VS, SHAPE_FS, [v4("i_x"), v4("i_y"), v4("i_fill"), v4("i_stroke"), v4("i_m"), v4("i_c")]);

/** A point in lane space: a beat plus a px offset, and px from the row top. */
export interface LanePoint {
  beat: number;
  px?: number;
  y: number;
}

/** Vertical clip range of an instance (px from the row top). */
export interface ClipY {
  y0: number;
  y1: number;
}

const TRANSPARENT: Rgba = [0, 0, 0, 0];

function put(
  w: InstanceWriterLike,
  a: LanePoint,
  b: LanePoint,
  radius: number,
  strokeWidth: number,
  fill: Rgba,
  stroke: Rgba,
  mode: number,
  dash: number,
  minW: number,
  alpha: number,
  clip: ClipY,
): void {
  const o = w.next();
  const d = w.data;
  d[o] = a.beat;
  d[o + 1] = a.px ?? 0;
  d[o + 2] = b.beat;
  d[o + 3] = b.px ?? 0;
  d[o + 4] = a.y;
  d[o + 5] = b.y;
  d[o + 6] = radius;
  d[o + 7] = strokeWidth;
  d.set(fill, o + 8);
  d.set(stroke, o + 12);
  d[o + 16] = mode;
  d[o + 17] = dash;
  d[o + 18] = minW;
  d[o + 19] = alpha;
  d[o + 20] = clip.y0;
  d[o + 21] = clip.y1;
  d[o + 22] = 0;
  d[o + 23] = 0;
}

/**
 * A capsule from `a` to `b` of radius `radius` (a line of width `2 * radius` with round
 * caps), or a circle when `a` = `b`. `strokeWidth` > 0 draws an inner stroke in `stroke`
 * over the fill (an SVG stroke centered on radius `r` with width `s` is radius `r + s / 2`
 * with an inner stroke of `s`).
 */
export function writeSegment(
  w: InstanceWriterLike,
  a: LanePoint,
  b: LanePoint,
  radius: number,
  fill: Rgba,
  clip: ClipY,
  stroke?: { color: Rgba; width: number },
  alpha = 1,
): void {
  put(w, a, b, radius, stroke?.width ?? 0, fill, stroke?.color ?? TRANSPARENT, 0, 0, 0, alpha, clip);
}

/**
 * The inner stroke (`width`) of the rounded rectangle from `a` (top-left) to `b`
 * (bottom-right, at least `minW` px wide): solid, or dashed with `dash` px dashes and gaps.
 */
export function writeOutline(
  w: InstanceWriterLike,
  a: LanePoint,
  b: LanePoint,
  o: { radius: number; width: number; color: Rgba; dash?: number; minW?: number; alpha?: number; clip: ClipY },
): void {
  put(w, a, b, o.radius, o.width, TRANSPARENT, o.color, 1, o.dash ?? 0, o.minW ?? 0, o.alpha ?? 1, o.clip);
}
