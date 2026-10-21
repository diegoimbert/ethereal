/**
 * The `fade` pipeline of the clip-editing layers: what `ClipFades` draws in its SVG and
 * handles, as instanced quads in lane space (see ../shaders.ts):
 *
 * - `Tint`: a crossfade region (a rect over the fade's span, the body's height);
 * - `Shade`: the region above a fade curve (what the fade removes);
 * - `Line` / `Dashed`: the fade curve (this clip's, or the crossfade neighbour's, dashed),
 *   a 1 px antialiased stroke;
 * - `Handle` / `CurveHandle`: the fade length handle (a square on the body's top edge) and
 *   the curve handle (a dot on the curve's midpoint), with a 1 px border;
 * - `Hatch` / `Outline`: a missing sample's clip (media-refs): 135° stripes (2 px every
 *   8 px) and a dashed 1 px outline inside the clip box.
 *
 * The curve is the fade law (`fadeGain`, Rust `ether_core::fades::fade_gain`) evaluated per
 * pixel, so it stays exact at any zoom without rebuilding instances. Every instance is
 * clipped to its clip's rounded box and to the span of the clip not covered by clips drawn
 * above it (`vis`), since layers draw after all the lane's clips.
 */

import type { FadeCurve } from "@/generated";
import { Pipeline } from "../glContext";
import type { Rgba } from "../palette";
import type { FxCode, InstanceWriter } from "../pipelines";
import { FRAG_COMMON, SHADER_COMMON } from "../shaders";
import { HANDLE_PX } from "./fadeModel";

export const FadeMode = { Tint: 0, Shade: 1, Line: 2, Dashed: 3, Handle: 4, CurveHandle: 5, Hatch: 6, Outline: 7 } as const;
export type FadeModeCode = (typeof FadeMode)[keyof typeof FadeMode];

/** `FadeCurve` as the shader's curve code and tension. */
export function curveCode(c: FadeCurve): [number, number] {
  switch (c.type) {
    case "Linear":
      return [0, 0];
    case "EqualPower":
      return [1, 0];
    case "Curve":
      return [2, c.tension];
  }
}

const VS = /* glsl */ `#version 300 es
precision highp float;
${SHADER_COMMON}
in vec2 a_corner;
in vec4 i_box;    // clip box beats x0, x1, min width, mode
in vec4 i_fade;   // fade span beats f0, f1 (handles: anchor beat, unused), curve code, tension
in vec4 i_vis;    // visible span beats v0, v1, direction (1: fade-in, -1: fade-out), radius
in vec4 i_fill;
in vec4 i_stroke;
in vec4 i_q;      // fx code, alpha, handle min px from the box start, from the box end
out vec2 v_pos;
flat out vec4 v_clip;
flat out vec4 v_span;   // visible x0, x1, fade x0, x1 (css px)
flat out vec4 v_body;   // body top, body height, mode, direction
flat out vec4 v_curve;  // curve code, tension, fx, alpha
flat out vec4 v_rect;   // handle rect
flat out float v_radius;
flat out vec4 v_fill;
flat out vec4 v_stroke;

const float HANDLE = ${HANDLE_PX.toFixed(1)};

float gain(float code, float tension, float x) {
  float t = clamp(x, 0.0, 1.0);
  if (code < 0.5) return t;
  if (code < 1.5) return sin(t * 1.5707963267948966);
  return pow(t, pow(4.0, clamp(tension, -1.0, 1.0)));
}

void main() {
  int mode = int(i_box.w + 0.5);
  vec2 bxr = boxX(vec4(i_box.x, i_box.y, 0.0, 0.0), i_box.z);
  vec4 clip = vec4(snapPx(bxr.x), snapPx(rowY(u_clip.x, 0.0)), snapPx(bxr.y), snapPx(rowY(-u_clip.x, 1.0)));
  float top = rowY(u_clip.x + u_clip.y, 0.0);
  float bodyH = max(0.0, u_row.y - 2.0 * u_clip.x - u_clip.y);
  float f0 = bx(i_fade.x);
  float f1 = bx(i_fade.y);
  float v0 = bx(i_vis.x);
  float v1 = bx(i_vis.y);
  vec4 r;
  bool show = bodyH > 0.0;
  if (mode == ${FadeMode.Handle}) {
    float cx = clamp(f0, bxr.x + i_q.z, bxr.y - i_q.w);
    r = vec4(cx - HANDLE * 0.5, top, cx + HANDLE * 0.5, top + HANDLE);
  } else if (mode == ${FadeMode.CurveHandle}) {
    float cx = 0.5 * (f0 + f1);
    float cy = top + (1.0 - gain(i_fade.z, i_fade.w, 0.5)) * bodyH;
    r = vec4(cx - HANDLE * 0.5, cy - HANDLE * 0.5, cx + HANDLE * 0.5, cy + HANDLE * 0.5);
    show = show && f1 - f0 >= 8.0;
  } else if (mode >= ${FadeMode.Hatch}) {
    r = clip;
  } else {
    r = vec4(f0, top, f1, top + bodyH);
    show = show && f1 - f0 > 0.5;
  }
  vec2 lo = max(r.xy - 1.0, vec2(max(clip.x, v0), clip.y) - 1.0);
  vec2 hi = min(r.zw + 1.0, vec2(min(clip.z, v1), clip.w) + 1.0);
  v_pos = mix(lo, hi, a_corner);
  gl_Position = show && hi.x > lo.x && hi.y > lo.y ? toClip(v_pos) : vec4(2.0, 2.0, 2.0, 1.0);
  v_clip = clip;
  v_span = vec4(v0, v1, f0, f1);
  v_body = vec4(top, bodyH, float(mode), i_vis.z);
  v_curve = vec4(i_fade.z, i_fade.w, i_q.x, i_q.y);
  v_rect = r;
  v_radius = i_vis.w;
  v_fill = i_fill;
  v_stroke = i_stroke;
}
`;

const FS = /* glsl */ `#version 300 es
precision highp float;
${FRAG_COMMON}
in vec2 v_pos;
flat in vec4 v_clip;
flat in vec4 v_span;
flat in vec4 v_body;
flat in vec4 v_curve;
flat in vec4 v_rect;
flat in float v_radius;
flat in vec4 v_fill;
flat in vec4 v_stroke;
out vec4 o_color;

const float CLIP_RADIUS = __RADIUS__;
const float DASH = 2.0;

float gain(float x) {
  float t = clamp(x, 0.0, 1.0);
  if (v_curve.x < 0.5) return t;
  if (v_curve.x < 1.5) return sin(t * 1.5707963267948966);
  return pow(t, pow(4.0, clamp(v_curve.y, -1.0, 1.0)));
}

// The fade curve's y (css px) at x: the gain from the top of the body (full level) down.
float curveY(float x) {
  float u = (x - v_span.z) / max(1e-6, v_span.w - v_span.z);
  float g = v_body.w > 0.0 ? gain(u) : gain(1.0 - u);
  return v_body.x + (1.0 - clamp(g, 0.0, 1.0)) * v_body.y;
}

float segDist(vec2 p, vec2 a, vec2 b) {
  vec2 ab = b - a;
  float t = clamp(dot(p - a, ab) / max(1e-6, dot(ab, ab)), 0.0, 1.0);
  return length(p - a - ab * t);
}

// Distance to the curve: to its polyline through the neighbouring pixel columns.
float curveDist(vec2 p) {
  float d = 1e6;
  vec2 prev = vec2(0.0);
  for (int i = 0; i < 5; i++) {
    float x = clamp(p.x + float(i) - 2.0, v_span.z, v_span.w);
    vec2 q = vec2(x, curveY(x));
    if (i > 0) d = min(d, segDist(p, prev, q));
    prev = q;
  }
  return d;
}

// Arc length of the curve from its start to x (css px), for the dash pattern.
float arcLength(float x) {
  float x1 = clamp(x, v_span.z, v_span.w);
  float s = 0.0;
  vec2 prev = vec2(v_span.z, curveY(v_span.z));
  for (int i = 1; i <= 32; i++) {
    float xi = mix(v_span.z, x1, float(i) / 32.0);
    vec2 q = vec2(xi, curveY(xi));
    s += length(q - prev);
    prev = q;
  }
  return s;
}

void main() {
  float clipCov = coverage(sdBox(v_pos, v_clip, CLIP_RADIUS));
  clipCov *= clamp((v_pos.x - v_span.x) * u_canvas.z + 0.5, 0.0, 1.0) * clamp((v_span.y - v_pos.x) * u_canvas.z + 0.5, 0.0, 1.0);
  if (clipCov <= 0.0) discard;
  int mode = int(v_body.z + 0.5);
  // Translucent over the waveform: the clip opacity (dragged) multiplies the alpha (below)
  // instead of pre-blending against the background as the clip body does.
  vec4 fill = clipFx(v_fill, v_curve.z, 1.0);
  vec4 o;
  if (mode == ${FadeMode.Tint}) {
    o = vec4(fill.rgb * fill.a, fill.a) * coverage(sdBox(v_pos, v_rect, 0.0));
  } else if (mode == ${FadeMode.Shade}) {
    float xc = clamp((v_pos.x - v_span.z) * u_canvas.z + 0.5, 0.0, 1.0) * clamp((v_span.w - v_pos.x) * u_canvas.z + 0.5, 0.0, 1.0);
    float yc = clamp((curveY(v_pos.x) - v_pos.y) * u_canvas.z + 0.5, 0.0, 1.0) * clamp((v_pos.y - v_body.x) * u_canvas.z + 0.5, 0.0, 1.0);
    o = vec4(fill.rgb * fill.a, fill.a) * xc * yc;
  } else if (mode == ${FadeMode.Line} || mode == ${FadeMode.Dashed}) {
    if (v_pos.x < v_span.z - 1.0 || v_pos.x > v_span.w + 1.0) discard;
    float d = curveDist(v_pos);
    float c = clamp((0.5 - d) * u_canvas.z + 0.5, 0.0, 1.0);
    if (c <= 0.0) discard;
    if (mode == ${FadeMode.Dashed} && mod(arcLength(v_pos.x), 2.0 * DASH) >= DASH) discard;
    o = vec4(fill.rgb * fill.a, fill.a) * c;
  } else if (mode == ${FadeMode.Hatch}) {
    // repeating-linear-gradient(135deg, c 0 2px, transparent 2px 8px)
    float t = (v_pos.x - v_clip.x + v_pos.y - v_clip.y) * 0.70710678;
    float m = mod(t, 8.0);
    float c = clamp((2.0 - m) * u_canvas.z + 0.5, 0.0, 1.0) * clamp(m * u_canvas.z + 0.5, 0.0, 1.0);
    o = vec4(fill.rgb * fill.a, fill.a) * c;
  } else if (mode == ${FadeMode.Outline}) {
    // outline: 1px dashed, offset -1px (inside the box, following its corners).
    float d = sdBox(v_pos, v_clip, CLIP_RADIUS);
    float c = max(0.0, coverage(d) - coverage(d + 1.0));
    vec2 rel = v_pos - v_clip.xy;
    float along = min(abs(v_pos.y - v_clip.y), abs(v_clip.w - v_pos.y)) < 1.5 ? rel.x : rel.y;
    if (mod(along, 6.0) >= 3.0) discard;
    o = vec4(fill.rgb * fill.a, fill.a) * c;
  } else {
    // Handles: a fill inside a 1 px border (box-sizing: border-box).
    float d = sdBox(v_pos, v_rect, v_radius);
    float c = coverage(d);
    vec4 stroke = clipFx(v_stroke, v_curve.z, 1.0);
    float sc = max(0.0, c - coverage(d + 1.0));
    o = vec4(stroke.rgb * stroke.a, stroke.a) * sc + vec4(fill.rgb * fill.a, fill.a) * (c - sc);
  }
  o_color = o * clipCov * v_curve.w;
  if (o_color.a <= 0.0) discard;
}
`;

export const FADE_STRIDE = 24;

/** The `fade` pipeline (`radius`: the clip boxes' corner radius). */
export function fadePipeline(radius: number): Pipeline {
  const v4 = (name: string) => ({ name, size: 4 });
  return new Pipeline(
    "fade",
    VS,
    FS.replace("__RADIUS__", radius.toFixed(2)),
    [v4("i_box"), v4("i_fade"), v4("i_vis"), v4("i_fill"), v4("i_stroke"), v4("i_q")],
    { radius },
  );
}

export interface FadeSpec {
  mode: FadeModeCode;
  /** The clip box (beats, min width px). */
  box: { b0: number; b1: number; minW: number };
  /** The fade's span (beats); for a length handle, `f0` is its anchor beat. */
  f0: number;
  f1: number;
  curve?: FadeCurve;
  /** 1: a fade-in (gain rises over the span), -1: a fade-out. */
  dir?: 1 | -1;
  /** Visible span (beats): the part of the clip no clip above covers. */
  vis: [number, number];
  fill: Rgba;
  stroke?: Rgba;
  fx?: FxCode;
  alpha?: number;
  /** Handles: corner radius, and how close to the box's start / end the handle may go (px). */
  radius?: number;
  minFromStart?: number;
  minFromEnd?: number;
}

const NONE = 1e6;

export function writeFade(w: InstanceWriter, s: FadeSpec): void {
  const o = w.next();
  const d = w.data;
  const [code, tension] = s.curve ? curveCode(s.curve) : [0, 0];
  d[o] = s.box.b0;
  d[o + 1] = s.box.b1;
  d[o + 2] = s.box.minW;
  d[o + 3] = s.mode;
  d[o + 4] = s.f0;
  d[o + 5] = s.f1;
  d[o + 6] = code;
  d[o + 7] = tension;
  d[o + 8] = s.vis[0];
  d[o + 9] = s.vis[1];
  d[o + 10] = s.dir ?? 1;
  d[o + 11] = s.radius ?? 0;
  d.set(s.fill, o + 12);
  d.set(s.stroke ?? [0, 0, 0, 0], o + 16);
  d[o + 20] = s.fx ?? 0;
  d[o + 21] = s.alpha ?? 1;
  d[o + 22] = s.minFromStart ?? -NONE;
  d[o + 23] = s.minFromEnd ?? -NONE;
}
