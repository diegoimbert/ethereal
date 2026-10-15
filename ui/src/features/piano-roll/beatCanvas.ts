/**
 * The beat editor's steps, drawn on one canvas (WebGL2, else a 2D context): every visible
 * step of every row as a rounded rect, in a single draw call. DOM elements per step made a
 * zoomed-out view (thousands of steps) lag.
 *
 * `beatRects` lays the steps out (pure: tested without a GPU); `BeatRenderer` draws them.
 * Coordinates are CSS px from the canvas's top left; colors `[r, g, b, a]` floats (straight).
 */

import type { Rgba } from "@/features/arrangement/gl/palette";
import { renderDpr } from "@/timeline/renderDpr";

export interface BeatRect {
  x: number;
  y: number;
  w: number;
  h: number;
  radius: number;
  color: Rgba;
}

export interface BeatColors {
  /** Text color: steps are tints of it (light on dark, dark on light). */
  text: Rgba;
  accent: Rgba;
  /** Bar start lines. */
  bar: Rgba;
}

export interface BeatLayout {
  /** Row keys (pitches), top to bottom. */
  rows: ReadonlyArray<number>;
  rowH: number;
  /** Visible step indices (from the region start; `>= steps`: past the clip's end). */
  visible: ReadonlyArray<number>;
  steps: number;
  /** x (CSS px) of step `i`'s left edge, and a step's width. */
  xOf(i: number): number;
  cellW: number;
  /** Velocity (0–1) of the hit at row `note`, step `i` (in the clip), or null. */
  hitAt(note: number, i: number): number | null;
  /** Repeat the pattern past the clip's end (the clip loops). */
  looping: boolean;
  /** Step `i` starts a beat with an odd index (shaded) / starts a bar (line). */
  alt(i: number): boolean;
  bar(i: number): boolean;
  hover: { note: number; i: number } | null;
  /** Insets (CSS px): a pad inside its slot, and its corner radius. */
  insetX: number;
  insetY: number;
  radius: number;
  barWidth: number;
}

/** Step tints (alpha of the text color), as the CSS version had them. */
const PAD_ALPHA = 0.09;
const ALT_ALPHA = 0.16;
const HOVER_ALPHA = 0.28;
/** Past the clip's end: everything at this opacity. */
const PAST_ALPHA = 0.3;

const tint = (c: Rgba, a: number): Rgba => [c[0], c[1], c[2], c[3] * a];

export function beatRects(l: BeatLayout, colors: BeatColors): BeatRect[] {
  const out: BeatRect[] = [];
  const w = Math.max(1, l.cellW - 2 * l.insetX);
  const h = Math.max(1, l.rowH - 2 * l.insetY);
  l.rows.forEach((note, r) => {
    const top = r * l.rowH;
    for (const i of l.visible) {
      const past = i >= l.steps;
      const fade = past ? PAST_ALPHA : 1;
      const x = l.xOf(i);
      if (l.bar(i)) out.push({ x, y: top, w: l.barWidth, h: l.rowH, radius: 0, color: tint(colors.bar, fade) });
      const velocity = past ? (l.looping ? l.hitAt(note, i % l.steps) : null) : l.hitAt(note, i);
      const hovered = !past && l.hover?.note === note && l.hover.i === i;
      const color =
        velocity !== null
          ? tint(colors.accent, (0.45 + 0.55 * velocity) * fade)
          : tint(colors.text, (hovered ? HOVER_ALPHA : l.alt(i) ? ALT_ALPHA : PAD_ALPHA) * fade);
      out.push({ x: x + l.insetX, y: top + l.insetY, w, h, radius: Math.min(l.radius, w / 2, h / 2), color });
    }
  });
  return out;
}

// ---- Drawing ----------------------------------------------------------------------------

const VS = /* glsl */ `#version 300 es
precision highp float;
in vec2 a_corner;
in vec4 i_rect;   // x, y, w, h (css px)
in vec4 i_color;  // straight rgba
in float i_radius;
uniform vec2 u_size; // canvas css px
out vec2 v_local;
flat out vec2 v_half;
flat out float v_radius;
flat out vec4 v_color;
void main() {
  // One device pixel of margin for the antialiased edge.
  vec2 grow = vec2(1.0);
  vec2 p = i_rect.xy - grow + a_corner * (i_rect.zw + 2.0 * grow);
  v_local = p - (i_rect.xy + 0.5 * i_rect.zw);
  v_half = 0.5 * i_rect.zw;
  v_radius = i_radius;
  v_color = i_color;
  vec2 clip = p / u_size * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
}
`;

const FS = /* glsl */ `#version 300 es
precision highp float;
uniform float u_dpr;
in vec2 v_local;
flat in vec2 v_half;
flat in float v_radius;
flat in vec4 v_color;
out vec4 o_color;
float sdRoundBox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
void main() {
  float d = sdRoundBox(v_local, v_half, v_radius);
  float a = v_color.a * clamp(0.5 - d * u_dpr, 0.0, 1.0);
  if (a <= 0.0) discard;
  o_color = vec4(v_color.rgb * a, a);
}
`;

/** Floats per instance: rect (4), color (4), radius (1). */
const STRIDE = 9;

interface Gl {
  gl: WebGL2RenderingContext;
  program: WebGLProgram;
  vao: WebGLVertexArrayObject;
  instances: WebGLBuffer;
  uSize: WebGLUniformLocation | null;
  uDpr: WebGLUniformLocation | null;
  capacity: number;
}

function compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader | null {
  const s = gl.createShader(type);
  if (!s) return null;
  gl.shaderSource(s, src);
  gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    console.warn("[beat editor] shader:", gl.getShaderInfoLog(s));
    return null;
  }
  return s;
}

function setupGl(canvas: HTMLCanvasElement): Gl | null {
  let gl: WebGL2RenderingContext | null = null;
  try {
    gl = canvas.getContext("webgl2", { premultipliedAlpha: true, antialias: false, alpha: true });
  } catch {
    return null;
  }
  if (!gl) return null;
  const vs = compile(gl, gl.VERTEX_SHADER, VS);
  const fs = compile(gl, gl.FRAGMENT_SHADER, FS);
  const program = gl.createProgram();
  if (!vs || !fs || !program) return null;
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return null;
  const vao = gl.createVertexArray();
  const corners = gl.createBuffer();
  const instances = gl.createBuffer();
  if (!vao || !corners || !instances) return null;
  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, corners);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
  const aCorner = gl.getAttribLocation(program, "a_corner");
  gl.enableVertexAttribArray(aCorner);
  gl.vertexAttribPointer(aCorner, 2, gl.FLOAT, false, 0, 0);
  gl.bindBuffer(gl.ARRAY_BUFFER, instances);
  const attr = (name: string, size: number, offset: number) => {
    const loc = gl.getAttribLocation(program, name);
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, size, gl.FLOAT, false, STRIDE * 4, offset * 4);
    gl.vertexAttribDivisor(loc, 1);
  };
  attr("i_rect", 4, 0);
  attr("i_color", 4, 4);
  attr("i_radius", 1, 8);
  gl.bindVertexArray(null);
  return { gl, program, vao, instances, uSize: gl.getUniformLocation(program, "u_size"), uDpr: gl.getUniformLocation(program, "u_dpr"), capacity: 0 };
}

/** Draws `BeatRect`s on a canvas: WebGL2 when available, else the 2D context. */
export class BeatRenderer {
  private gl: Gl | null;
  private ctx: CanvasRenderingContext2D | null = null;
  private data = new Float32Array(0);

  constructor(private canvas: HTMLCanvasElement) {
    this.gl = setupGl(canvas);
    if (!this.gl) {
      try {
        this.ctx = canvas.getContext("2d");
      } catch {
        this.ctx = null;
      }
    }
  }

  /** Size the backing store to the canvas's CSS size × the device pixel ratio, then draw. */
  draw(rects: ReadonlyArray<BeatRect>, cssW: number, cssH: number): void {
    const dpr = renderDpr();
    const w = Math.max(1, Math.round(cssW * dpr));
    const h = Math.max(1, Math.round(cssH * dpr));
    if (this.canvas.width !== w) this.canvas.width = w;
    if (this.canvas.height !== h) this.canvas.height = h;
    if (this.gl) this.drawGl(this.gl, rects, cssW, cssH, dpr);
    else if (this.ctx) this.draw2d(this.ctx, rects, dpr);
  }

  private drawGl(g: Gl, rects: ReadonlyArray<BeatRect>, cssW: number, cssH: number, dpr: number): void {
    const { gl } = g;
    const need = rects.length * STRIDE;
    if (this.data.length < need) this.data = new Float32Array(Math.max(need, this.data.length * 2));
    const d = this.data;
    rects.forEach((r, k) => {
      const o = k * STRIDE;
      d[o] = r.x;
      d[o + 1] = r.y;
      d[o + 2] = r.w;
      d[o + 3] = r.h;
      d[o + 4] = r.color[0];
      d[o + 5] = r.color[1];
      d[o + 6] = r.color[2];
      d[o + 7] = r.color[3];
      d[o + 8] = r.radius;
    });
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (!rects.length) return;
    gl.useProgram(g.program);
    gl.uniform2f(g.uSize, cssW, cssH);
    gl.uniform1f(g.uDpr, dpr);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.bindBuffer(gl.ARRAY_BUFFER, g.instances);
    if (g.capacity < need) {
      gl.bufferData(gl.ARRAY_BUFFER, this.data, gl.DYNAMIC_DRAW);
      g.capacity = this.data.length;
    } else {
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, d, 0, need);
    }
    gl.bindVertexArray(g.vao);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, rects.length);
    gl.bindVertexArray(null);
  }

  private draw2d(ctx: CanvasRenderingContext2D, rects: ReadonlyArray<BeatRect>, dpr: number): void {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    for (const r of rects) {
      const [cr, cg, cb, ca] = r.color;
      // eth-allow-hardcoded: a computed theme color, serialized for the 2D context
      ctx.fillStyle = `rgba(${Math.round(cr * 255)}, ${Math.round(cg * 255)}, ${Math.round(cb * 255)}, ${ca})`;
      if (r.radius > 0 && typeof ctx.roundRect === "function") {
        ctx.beginPath();
        ctx.roundRect(r.x, r.y, r.w, r.h, r.radius);
        ctx.fill();
      } else {
        ctx.fillRect(r.x, r.y, r.w, r.h);
      }
    }
  }

  dispose(): void {
    const g = this.gl;
    if (!g) return;
    g.gl.deleteBuffer(g.instances);
    g.gl.deleteVertexArray(g.vao);
    g.gl.deleteProgram(g.program);
    this.gl = null;
  }
}
