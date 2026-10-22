// eth-allow-case-file: GLSL source.
/**
 * GLSL (WebGL2, `#version 300 es`) of the arranger pipelines. Every pipeline draws
 * instanced quads; instance positions are in "lane space":
 *
 * - x: a beat plus a px offset, `x = (beat - scroll) * ppb + px + originX`, so pan and zoom
 *   only change uniforms (instances are never rebuilt for them);
 * - y: px from the top of the instance's row: `y = rowY + a + b * rowH` (`rowY`/`rowH` are
 *   per-draw uniforms: the lane's top in content px and its main height), so lanes resizing
 *   or moving (automation lanes opening above) don't rebuild them either.
 *
 * Coordinates in the fragment shaders are CSS px from the canvas's top-left.
 */

/** Shared uniforms and helpers of the vertex shaders. */
export const SHADER_COMMON = /* glsl */ `
uniform vec4 u_view;   // scroll beats (hi, lo), px per beat, x of the lane origin (css px)
uniform vec4 u_canvas; // canvas width, height (css px), device pixel ratio, content y at the canvas top
uniform vec2 u_row;    // row top (content px), main lane height (px)
uniform vec2 u_clip;   // clip inset in its lane, title band height (px)

float bx(float b) { return ((b - u_view.x) - u_view.y) * u_view.z + u_view.w; }
float rowY(float a, float b) { return u_row.x - u_canvas.w + a + b * u_row.y; }
vec4 toClip(vec2 p) { return vec4(p.x / u_canvas.x * 2.0 - 1.0, 1.0 - p.y / u_canvas.y * 2.0, 0.0, 1.0); }
float snapPx(float v) { return floor(v * u_canvas.z + 0.5) / u_canvas.z; }
// A clip box's x extent: [x0, max(x0 + minW, x1)].
vec2 boxX(vec4 x, float minW) { float x0 = bx(x.x) + x.z; return vec2(x0, max(x0 + minW, bx(x.y) + x.w)); }
`;

/** Shared helpers of the fragment shaders. */
export const FRAG_COMMON = /* glsl */ `
uniform vec4 u_canvas;
uniform vec3 u_bg;
float coverage(float d) { return clamp(0.5 - d * u_canvas.z, 0.0, 1.0); }
float sdBox(vec2 p, vec4 r, float rad) {
  vec2 c = (r.xy + r.zw) * 0.5;
  vec2 h = max((r.zw - r.xy) * 0.5, vec2(0.0));
  rad = min(rad, min(h.x, h.y));
  vec2 q = abs(p - c) - h + rad;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rad;
}
// CSS filter: saturate(s).
vec3 saturateCss(vec3 c, float s) {
  return vec3(
    dot(c, vec3(0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s)),
    dot(c, vec3(0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s)),
    dot(c, vec3(0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s)));
}
// Clip effects (fx code): 1 = muted (saturate .15, opacity .55), 2 = frozen track (.35, .75),
// 3 = a track still loading (saturate .2; its opacity is the lane's, eased back as it loads),
// times the clip's own opacity (dragged .85, ghost .45). CSS applies the opacity to the clip
// as one layer: here each part is mixed into the arranger background instead, which is the
// same over the lane background (parts of one clip don't show through each other, as they
// would with per-instance alpha).
vec4 clipFx(vec4 c, float fx, float opacity) {
  float o = opacity;
  vec3 rgb = c.rgb;
  if (fx > 0.5) {
    rgb = saturateCss(rgb, fx < 1.5 ? 0.15 : fx < 2.5 ? 0.35 : 0.2);
    o *= fx < 1.5 ? 0.55 : fx < 2.5 ? 0.75 : 1.0;
  }
  if (o < 1.0) rgb = mix(u_bg, rgb, o);
  return vec4(rgb, c.a);
}
`;

export const RECT_VS = /* glsl */ `#version 300 es
precision highp float;
${SHADER_COMMON}
in vec2 a_corner;
in vec4 i_x;      // beats x0, x1, px offsets x0, x1
in vec4 i_p;      // min width, radius, blur, flags
in vec4 i_y;      // y0 = a0 + b0 * rowH, y1 = a1 + b1 * rowH
in vec4 i_fill;
in vec4 i_stroke;
in vec4 i_q;      // stroke width, clip min width, clip radius, alpha
in vec4 i_cx;     // clip box x (as i_x)
in vec4 i_cy;     // clip box y (as i_y)
out vec2 v_pos;
flat out vec4 v_rect;
flat out vec4 v_clipRect;
flat out vec4 v_fill;
flat out vec4 v_stroke;
flat out vec4 v_params; // radius, blur, stroke width, clip radius
flat out vec2 v_fx;     // fx code, alpha

const int HANDLE_START = 1;
const int HANDLE_END = 2;
const int CLIP_LANE = 4;
const int NO_CLIP = 8;
const int SNAP = 16;
const int FX_SHIFT = 5;

void main() {
  int flags = int(i_p.w + 0.5);
  vec2 cx = boxX(i_cx, i_q.y);
  vec4 clipRect = vec4(cx.x, rowY(i_cy.x, i_cy.y), cx.y, rowY(i_cy.z, i_cy.w));
  vec2 x = boxX(i_x, i_p.x);
  float handle = clamp(0.25 * (cx.y - cx.x), 3.0, 6.0); // hitTest.ts handleWidth
  if ((flags & HANDLE_START) != 0) x = vec2(cx.x, cx.x + handle);
  if ((flags & HANDLE_END) != 0) x = vec2(cx.y - handle, cx.y);
  vec4 r = vec4(x.x, rowY(i_y.x, i_y.y), x.y, rowY(i_y.z, i_y.w));
  if ((flags & SNAP) != 0) {
    r = vec4(snapPx(r.x), snapPx(r.y), snapPx(r.z), snapPx(r.w));
    clipRect = vec4(snapPx(clipRect.x), snapPx(clipRect.y), snapPx(clipRect.z), snapPx(clipRect.w));
  }
  if ((flags & CLIP_LANE) != 0) clipRect = vec4(-1e6, rowY(0.0, 0.0), 1e6, rowY(0.0, 1.0));
  if ((flags & NO_CLIP) != 0) clipRect = vec4(-1e6, -1e6, 1e6, 1e6);
  float grow = i_p.z > 0.0 ? i_p.z * 1.5 + 1.0 : 1.0;
  vec2 lo = max(r.xy - grow, clipRect.xy - 1.0);
  vec2 hi = min(r.zw + grow, clipRect.zw + 1.0);
  v_pos = mix(lo, hi, a_corner);
  gl_Position = hi.x > lo.x && hi.y > lo.y ? toClip(v_pos) : vec4(2.0, 2.0, 2.0, 1.0);
  v_rect = r;
  v_clipRect = clipRect;
  v_fill = i_fill;
  v_stroke = i_stroke;
  v_params = vec4(i_p.y, i_p.z, i_q.x, i_q.z);
  v_fx = vec2(float(flags >> FX_SHIFT), i_q.w);
}
`;

export const RECT_FS = /* glsl */ `#version 300 es
precision highp float;
${FRAG_COMMON}
in vec2 v_pos;
flat in vec4 v_rect;
flat in vec4 v_clipRect;
flat in vec4 v_fill;
flat in vec4 v_stroke;
flat in vec4 v_params;
flat in vec2 v_fx;
out vec4 o_color;

float erf(float x) {
  float s = sign(x), a = abs(x);
  x = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
  x *= x;
  return s - s / (x * x);
}

void main() {
  float clipCov = coverage(sdBox(v_pos, v_clipRect, v_params.w));
  if (clipCov <= 0.0) discard;
  float d = sdBox(v_pos, v_rect, v_params.x);
  vec4 o;
  if (v_params.y > 0.0) {
    // A CSS box-shadow blur of B px (a Gaussian of sigma B / 2), outside the box only, faded
    // with the clip's opacity (it is part of the clip's layer, over the lane).
    vec4 fill = clipFx(v_fill, v_fx.x, 1.0);
    float sigma = v_params.y * 0.5;
    float a = fill.a * v_fx.y * (0.5 - 0.5 * erf(d / (sigma * 1.41421356))) * (1.0 - coverage(d));
    o = vec4(fill.rgb * a, a);
  } else {
    vec4 fill = clipFx(v_fill, v_fx.x, v_fx.y);
    float c = coverage(d);
    o = vec4(fill.rgb * fill.a, fill.a) * c;
    if (v_params.z > 0.0) {
      vec4 stroke = clipFx(v_stroke, v_fx.x, v_fx.y);
      float sc = max(0.0, c - coverage(d + v_params.z));
      o = vec4(stroke.rgb * stroke.a, stroke.a) * sc + o * (1.0 - stroke.a * sc);
    }
  }
  o_color = o * clipCov;
  if (o_color.a <= 0.0) discard;
}
`;

export const TEXT_VS = /* glsl */ `#version 300 es
precision highp float;
${SHADER_COMMON}
in vec2 a_corner;
in vec4 i_pos;    // anchor beat, px offset x, y offset from the row top, unused
in vec4 i_size;   // width, height (css px), mode (0 text, 1 icon), fx code
in vec4 i_uv;     // atlas rect (texels)
in vec4 i_color;
in vec4 i_cx;     // horizontal clip (as a clip box x)
in vec4 i_c2;     // clip min width, alpha, unused, unused
out vec2 v_uv;
out vec2 v_pos;
flat out vec4 v_color;
flat out vec2 v_clipX;
flat out vec3 v_mode; // mode, fx, alpha

void main() {
  vec2 cx = boxX(i_cx, i_c2.x);
  float x0 = snapPx(bx(i_pos.x) + i_pos.y);
  float y0 = snapPx(rowY(i_pos.z, 0.0));
  v_pos = vec2(x0, y0) + a_corner * i_size.xy;
  v_uv = mix(i_uv.xy, i_uv.zw, a_corner);
  gl_Position = x0 < cx.y && x0 + i_size.x > cx.x ? toClip(v_pos) : vec4(2.0, 2.0, 2.0, 1.0);
  v_color = i_color;
  v_clipX = cx;
  v_mode = vec3(i_size.z, i_size.w, i_c2.y);
}
`;

export const TEXT_FS = /* glsl */ `#version 300 es
precision highp float;
${FRAG_COMMON}
uniform sampler2D u_atlas;
in vec2 v_uv;
in vec2 v_pos;
flat in vec4 v_color;
flat in vec2 v_clipX;
flat in vec3 v_mode;
out vec4 o_color;

// Sprites (text runs the browser rasterized, icons) are alpha coverage drawn 1:1 at device
// pixels: each fragment takes its texel, no filtering. The coverage blends in sRGB space
// (premultiplied, no linearization), as the browser composites text.
void main() {
  float clipCov = clamp((v_pos.x - v_clipX.x) * u_canvas.z + 0.5, 0.0, 1.0) * clamp((v_clipX.y - v_pos.x) * u_canvas.z + 0.5, 0.0, 1.0);
  float a = texelFetch(u_atlas, ivec2(floor(v_uv)), 0).r;
  vec4 c = clipFx(v_color, v_mode.y, v_mode.z);
  a *= c.a * clipCov;
  if (a <= 0.0) discard;
  o_color = vec4(c.rgb * a, a);
}
`;

export const NOTE_VS = /* glsl */ `#version 300 es
precision highp float;
${SHADER_COMMON}
in vec2 a_corner;
in vec4 i_n;      // start, end (beats), pitch position (0 = lowest row, 1 = highest), pitch rows (negative: drum rows)
in vec4 i_c;      // clip box beats x0, x1, min width, fx code
in vec4 i_color;  // ink, alpha = note opacity
in vec4 i_o;      // clip opacity (x)
out vec2 v_pos;
flat out vec4 v_rect;
flat out vec4 v_clipRect;
flat out vec4 v_color;
flat out vec2 v_fx;

void main() {
  float top = rowY(u_clip.x + u_clip.y, 0.0);
  float bodyH = u_row.y - 2.0 * u_clip.x - u_clip.y;
  // Drum rows (negative count): taller hits with a gap between rows, the body padded.
  bool drums = i_n.w < 0.0;
  float rows = abs(i_n.w);
  float pad = drums ? min(4.0, bodyH * 0.1) : 0.0;
  float h = drums ? max(1.0, min(14.0, (bodyH - 2.0 * pad) / rows * 0.6)) : max(1.0, min(6.0, bodyH / rows));
  float usable = bodyH - 2.0 * pad - h;
  float y = top + pad + usable - i_n.z * usable;
  float x0 = bx(i_n.x);
  float w = max(1.0, (i_n.y - i_n.x) * u_view.z - 1.0);
  vec2 cx = boxX(vec4(i_c.x, i_c.y, 0.0, 0.0), i_c.z);
  v_rect = vec4(x0, y, x0 + w, y + h);
  v_clipRect = vec4(cx.x, top, cx.y, top + max(0.0, bodyH));
  vec2 lo = max(v_rect.xy - 1.0, v_clipRect.xy);
  vec2 hi = min(v_rect.zw + 1.0, v_clipRect.zw);
  v_pos = mix(lo, hi, a_corner);
  gl_Position = bodyH > 0.0 && hi.x > lo.x && hi.y > lo.y ? toClip(v_pos) : vec4(2.0, 2.0, 2.0, 1.0);
  v_color = i_color;
  v_fx = vec2(i_c.w, i_o.x);
}
`;

export const NOTE_FS = /* glsl */ `#version 300 es
precision highp float;
${FRAG_COMMON}
in vec2 v_pos;
flat in vec4 v_rect;
flat in vec4 v_clipRect;
flat in vec4 v_color;
flat in vec2 v_fx;
out vec4 o_color;

void main() {
  float a = coverage(sdBox(v_pos, v_rect, 0.0)) * coverage(sdBox(v_pos, v_clipRect, 0.0));
  vec4 c = clipFx(v_color, v_fx.x, v_fx.y);
  a *= c.a;
  if (a <= 0.0) discard;
  o_color = vec4(c.rgb * a, a);
}
`;

export const WAVE_VS = /* glsl */ `#version 300 es
precision highp float;
${SHADER_COMMON}
in vec2 a_corner;
in vec4 i_a;      // beats t0, t1, peak index at t0, at t1 (in the tile's row)
in vec4 i_b;      // tile row, linear gain, fx code, alpha
in vec4 i_c;      // clip box beats x0, x1, min width, unused
in vec4 i_color;
out vec2 v_pos;
flat out vec4 v_span;   // x0, x1, u0, u1
flat out vec4 v_body;   // body top, body height, clip x0, clip x1
flat out vec4 v_b;
flat out vec4 v_color;

void main() {
  float top = rowY(u_clip.x + u_clip.y, 0.0);
  float bodyH = u_row.y - 2.0 * u_clip.x - u_clip.y;
  float x0 = bx(i_a.x);
  float x1 = bx(i_a.y);
  vec2 cx = boxX(vec4(i_c.x, i_c.y, 0.0, 0.0), i_c.z);
  vec2 lo = vec2(max(x0, cx.x), top);
  vec2 hi = vec2(min(x1, cx.y), top + bodyH);
  v_pos = mix(lo, hi, a_corner);
  gl_Position = bodyH > 0.0 && hi.x > lo.x ? toClip(v_pos) : vec4(2.0, 2.0, 2.0, 1.0);
  v_span = vec4(x0, x1, i_a.z, i_a.w);
  v_body = vec4(top, bodyH, cx);
  v_b = i_b;
  v_color = i_color;
}
`;

export const WAVE_FS = /* glsl */ `#version 300 es
precision highp float;
precision highp sampler2D;
${FRAG_COMMON}
uniform sampler2D u_peaks;
uniform vec4 u_over;
in vec2 v_pos;
flat in vec4 v_span;
flat in vec4 v_body;
flat in vec4 v_b;
flat in vec4 v_color;
out vec4 o_color;

void main() {
  float span = max(1e-6, v_span.y - v_span.x);
  float du = (v_span.w - v_span.z) / span;          // peaks per css px
  float u = v_span.z + (v_pos.x - v_span.x) * du;
  float half_ = 0.5 * abs(du) / u_canvas.z;         // half a device pixel, in peaks
  float ua = u - half_;
  float ub = u + half_;
  int i0 = clamp(int(floor(ua)), 0, 1023);
  int i1 = clamp(int(floor(ub)), 0, 1023);
  int row = int(v_b.x + 0.5);
  int n = i1 - i0 + 1;
  int step_ = max(1, n / 24);
  float mn = 1e9;
  float mx = -1e9;
  for (int i = i0; i <= i1; i += step_) {
    vec2 t = texelFetch(u_peaks, ivec2(i, row), 0).rg;
    mn = min(mn, t.x);
    mx = max(mx, t.y);
  }
  if (mx < mn) discard;
  float g = v_b.y;
  float hiV = mx * g;
  float loV = mn * g;
  float mid = v_body.y * 0.5;
  float y = v_pos.y - v_body.x;
  float top = mid - clamp(hiV, -1.0, 1.0) * mid;
  float bottom = mid - clamp(loV, -1.0, 1.0) * mid;
  bottom = max(bottom, top + 1.0);
  float cov = clamp((y - top) * u_canvas.z + 0.5, 0.0, 1.0) * clamp((bottom - y) * u_canvas.z + 0.5, 0.0, 1.0);
  vec4 c = v_color;
  // Full-scale marks: where the scaled peaks run past full scale (2 px at the lane edge).
  float mark = min(2.0, v_body.y * 0.5);
  if ((hiV > 1.0 && y < mark) || (loV < -1.0 && y > v_body.y - mark)) { c = u_over; cov = 1.0; }
  c = clipFx(c, v_b.z, v_b.w);
  // Inside the clip's rounded box (its bottom corners).
  cov *= coverage(sdBox(v_pos, vec4(v_body.z, v_body.x - 20.0, v_body.w, v_body.x + v_body.y), 6.0));
  float a = c.a * cov;
  if (a <= 0.0) discard;
  o_color = vec4(c.rgb * a, a);
}
`;
