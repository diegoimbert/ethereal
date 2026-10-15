/**
 * The piano roll's canvas renderer: rounded rects with an optional border (solid or dashed,
 * with its own top edge color) or a soft outer glow (a CSS `box-shadow: 0 0 <blur>`), drawn
 * in one instanced WebGL2 draw call per layer. Falls back to a 2D context (no glow) where
 * WebGL2 is unavailable (e.g. jsdom).
 *
 * `InstanceList` is the CPU side (a growable Float32Array, filled by `noteLayout.ts`);
 * `RectRenderer` draws one onto a canvas. Coordinates are CSS px from the canvas's top left;
 * colors `[r, g, b, a]` floats (straight alpha).
 */

import type { Rgba } from "@/features/arrangement/gl/palette";
import { renderDpr } from "@/timeline/renderDpr";

/** Floats per instance: rect (4), fill (4), border (4), top border (4), params (4), opacity (1). */
const STRIDE = 21;

const NONE: Rgba = [0, 0, 0, 0];

export interface RectStyle {
  fill: Rgba;
  /** Border color (inside the rect, like `box-sizing: border-box`). */
  border?: Rgba | null;
  /** Top edge of the border (e.g. a scale root's), else `border`. */
  top?: Rgba | null;
  radius?: number;
  borderWidth?: number;
  /** > 0: this instance is an outer glow of the rect (`box-shadow: 0 0 blur fill`), not a fill. */
  blur?: number;
  /** > 0: dashed border with dashes (and gaps) this long. */
  dash?: number;
  /** Whole-element opacity (applied after the border is composited over the fill). */
  opacity?: number;
}

/** Instances to draw, in paint order. Reused frame to frame (`clear`), so nothing is allocated. */
export class InstanceList {
  data = new Float32Array(STRIDE * 256);
  count = 0;

  clear(): void {
    this.count = 0;
  }

  push(x: number, y: number, w: number, h: number, s: RectStyle): void {
    const need = (this.count + 1) * STRIDE;
    if (this.data.length < need) {
      const next = new Float32Array(Math.max(need, this.data.length * 2));
      next.set(this.data);
      this.data = next;
    }
    const d = this.data;
    const o = this.count * STRIDE;
    const border = s.border ?? NONE;
    const top = s.top ?? border;
    d[o] = x;
    d[o + 1] = y;
    d[o + 2] = w;
    d[o + 3] = h;
    d.set(s.fill, o + 4);
    d.set(border, o + 8);
    d.set(top, o + 12);
    d[o + 16] = Math.max(0, Math.min(s.radius ?? 0, w / 2, h / 2));
    d[o + 17] = s.border ? (s.borderWidth ?? 1) : 0;
    d[o + 18] = s.blur ?? 0;
    d[o + 19] = s.dash ?? 0;
    d[o + 20] = s.opacity ?? 1;
    this.count++;
  }

  /** Instance `i` read back (tests, the 2D fallback). */
  at(i: number): DrawnRect {
    const d = this.data;
    const o = i * STRIDE;
    const c = (k: number): Rgba => [d[o + k]!, d[o + k + 1]!, d[o + k + 2]!, d[o + k + 3]!];
    return {
      x: d[o]!,
      y: d[o + 1]!,
      w: d[o + 2]!,
      h: d[o + 3]!,
      fill: c(4),
      border: c(8),
      top: c(12),
      radius: d[o + 16]!,
      borderWidth: d[o + 17]!,
      blur: d[o + 18]!,
      dash: d[o + 19]!,
      opacity: d[o + 20]!,
    };
  }
}

export interface DrawnRect {
  x: number;
  y: number;
  w: number;
  h: number;
  fill: Rgba;
  border: Rgba;
  top: Rgba;
  radius: number;
  borderWidth: number;
  blur: number;
  dash: number;
  opacity: number;
}

// ---- WebGL2 -----------------------------------------------------------------------------

const VS = /* glsl */ `#version 300 es
precision highp float;
in vec2 a_corner;
in vec4 i_rect;    // x, y, w, h (css px)
in vec4 i_fill;
in vec4 i_border;
in vec4 i_top;
in vec4 i_params;  // radius, border width, blur, dash
in float i_opacity;
uniform vec2 u_size; // canvas css px
out vec2 v_local;
flat out vec2 v_half;
flat out vec4 v_fill;
flat out vec4 v_border;
flat out vec4 v_top;
flat out vec4 v_params;
flat out float v_opacity;
void main() {
  // A px of margin for the antialiased edge; a glow reaches ~3 sigma (1.5 blur) out.
  vec2 grow = vec2(1.0 + 1.5 * i_params.z);
  vec2 p = i_rect.xy - grow + a_corner * (i_rect.zw + 2.0 * grow);
  v_local = p - (i_rect.xy + 0.5 * i_rect.zw);
  v_half = 0.5 * i_rect.zw;
  v_fill = i_fill;
  v_border = i_border;
  v_top = i_top;
  v_params = i_params;
  v_opacity = i_opacity;
  vec2 clip = p / u_size * 2.0 - 1.0;
  gl_Position = vec4(clip.x, -clip.y, 0.0, 1.0);
}
`;

const FS = /* glsl */ `#version 300 es
precision highp float;
uniform float u_dpr;
in vec2 v_local;
flat in vec2 v_half;
flat in vec4 v_fill;
flat in vec4 v_border;
flat in vec4 v_top;
flat in vec4 v_params;
flat in float v_opacity;
out vec4 o_color;
float sdRoundBox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
// erf (Abramowitz & Stegun 7.1.26-style approximation, |error| < 2e-3: plenty for a glow).
float erf1(float x) {
  float s = sign(x);
  float a = abs(x);
  float x2 = a * a;
  return s * sqrt(1.0 - exp(-x2 * (1.2732395 + 0.147 * x2) / (1.0 + 0.147 * x2)));
}
void main() {
  float radius = v_params.x;
  float bw = v_params.y;
  float blur = v_params.z;
  float dash = v_params.w;
  float d = sdRoundBox(v_local, v_half, radius);
  vec4 c;
  float a;
  if (blur > 0.0) {
    // box-shadow: a gaussian of sigma = blur / 2, only outside the box.
    float sigma = 0.5 * blur;
    float g = 0.5 - 0.5 * erf1(d / (sigma * 1.4142136));
    float outside = clamp(0.5 + d * u_dpr, 0.0, 1.0);
    c = v_fill;
    a = c.a * g * outside;
  } else {
    c = v_fill;
    if (bw > 0.0) {
      float band = clamp(0.5 + (d + bw) * u_dpr, 0.0, 1.0);
      bool topEdge = v_local.y < -v_half.y + bw;
      vec4 b = topEdge ? v_top : v_border;
      if (dash > 0.0) {
        bool horizontal = abs(v_local.y) > v_half.y - bw;
        float along = horizontal ? v_local.x + v_half.x : v_local.y + v_half.y;
        b.a *= step(mod(along, 2.0 * dash), dash);
      }
      float ba = b.a * band;
      float oa = ba + c.a * (1.0 - ba);
      vec3 rgb = oa > 0.0 ? (b.rgb * ba + c.rgb * c.a * (1.0 - ba)) / oa : vec3(0.0);
      c = vec4(rgb, oa);
    }
    a = c.a * clamp(0.5 - d * u_dpr, 0.0, 1.0);
  }
  a *= v_opacity;
  if (a <= 0.0) discard;
  o_color = vec4(c.rgb * a, a);
}
`;

interface Gl {
  gl: WebGL2RenderingContext;
  program: WebGLProgram;
  vao: WebGLVertexArrayObject;
  instances: WebGLBuffer;
  corners: WebGLBuffer;
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
    console.warn("[piano roll] shader:", gl.getShaderInfoLog(s));
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
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    console.warn("[piano roll] program:", gl.getProgramInfoLog(program));
    return null;
  }
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
    if (loc < 0) return;
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, size, gl.FLOAT, false, STRIDE * 4, offset * 4);
    gl.vertexAttribDivisor(loc, 1);
  };
  attr("i_rect", 4, 0);
  attr("i_fill", 4, 4);
  attr("i_border", 4, 8);
  attr("i_top", 4, 12);
  attr("i_params", 4, 16);
  attr("i_opacity", 1, 20);
  gl.bindVertexArray(null);
  return {
    gl,
    program,
    vao,
    instances,
    corners,
    uSize: gl.getUniformLocation(program, "u_size"),
    uDpr: gl.getUniformLocation(program, "u_dpr"),
    capacity: 0,
  };
}

/** Draws an `InstanceList` on a canvas: WebGL2 when available, else the 2D context. */
export class RectRenderer {
  private gl: Gl | null;
  private ctx: CanvasRenderingContext2D | null = null;
  /** The last frame drawn: redrawn when a lost WebGL context comes back. */
  private last: { list: InstanceList; cssW: number; cssH: number } | null = null;
  private onLost = (e: Event) => e.preventDefault();
  private onRestored = () => {
    this.gl = setupGl(this.canvas);
    if (this.last) this.draw(this.last.list, this.last.cssW, this.last.cssH);
  };

  constructor(private canvas: HTMLCanvasElement) {
    this.gl = setupGl(canvas);
    if (!this.gl) {
      try {
        this.ctx = canvas.getContext("2d");
      } catch {
        this.ctx = null;
      }
    } else {
      // The GPU may drop the context (sleep, driver reset): ask for it back, rebuild, redraw.
      canvas.addEventListener("webglcontextlost", this.onLost);
      canvas.addEventListener("webglcontextrestored", this.onRestored);
    }
  }

  /** Size the backing store to `cssW × cssH` × the device pixel ratio, then draw. */
  draw(list: InstanceList, cssW: number, cssH: number): void {
    this.last = { list, cssW, cssH };
    const dpr = renderDpr();
    const w = Math.max(1, Math.round(cssW * dpr));
    const h = Math.max(1, Math.round(cssH * dpr));
    if (this.canvas.width !== w) this.canvas.width = w;
    if (this.canvas.height !== h) this.canvas.height = h;
    if (this.gl) this.drawGl(this.gl, list, cssW, cssH, dpr);
    else if (this.ctx) this.draw2d(this.ctx, list, dpr);
  }

  private drawGl(g: Gl, list: InstanceList, cssW: number, cssH: number, dpr: number): void {
    const { gl } = g;
    if (gl.isContextLost()) return;
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (!list.count) return;
    const need = list.count * STRIDE;
    gl.useProgram(g.program);
    gl.uniform2f(g.uSize, cssW, cssH);
    gl.uniform1f(g.uDpr, dpr);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.bindBuffer(gl.ARRAY_BUFFER, g.instances);
    if (g.capacity < need) {
      // Grow to the list's capacity (it doubles), so a growing scene re-allocates rarely.
      gl.bufferData(gl.ARRAY_BUFFER, list.data, gl.DYNAMIC_DRAW);
      g.capacity = list.data.length;
    } else {
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, list.data, 0, need);
    }
    gl.bindVertexArray(g.vao);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, list.count);
    gl.bindVertexArray(null);
  }

  /** The fallback: fills, borders (dashes too) and opacity; glows are skipped. */
  private draw2d(ctx: CanvasRenderingContext2D, list: InstanceList, dpr: number): void {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
    // eth-allow-hardcoded: a computed theme color, formatted for the 2D context
    const css = (c: Rgba) => `rgba(${Math.round(c[0] * 255)}, ${Math.round(c[1] * 255)}, ${Math.round(c[2] * 255)}, ${c[3]})`;
    const path = (x: number, y: number, w: number, h: number, r: number) => {
      ctx.beginPath();
      if (r > 0 && typeof ctx.roundRect === "function") ctx.roundRect(x, y, w, h, r);
      else ctx.rect(x, y, w, h);
    };
    for (let i = 0; i < list.count; i++) {
      const r = list.at(i);
      if (r.blur > 0) continue;
      ctx.globalAlpha = r.opacity;
      path(r.x, r.y, r.w, r.h, r.radius);
      ctx.fillStyle = css(r.fill);
      ctx.fill();
      if (r.borderWidth > 0) {
        const b = r.borderWidth;
        ctx.lineWidth = b;
        ctx.setLineDash(r.dash > 0 ? [r.dash, r.dash] : []);
        ctx.strokeStyle = css(r.border);
        path(r.x + b / 2, r.y + b / 2, r.w - b, r.h - b, Math.max(0, r.radius - b / 2));
        ctx.stroke();
        if (r.top.some((v, k) => v !== r.border[k])) {
          ctx.fillStyle = css(r.top);
          ctx.fillRect(r.x + r.radius, r.y, r.w - 2 * r.radius, b);
        }
      }
    }
    ctx.globalAlpha = 1;
    ctx.setLineDash([]);
  }

  dispose(): void {
    this.canvas.removeEventListener("webglcontextlost", this.onLost);
    this.canvas.removeEventListener("webglcontextrestored", this.onRestored);
    this.last = null;
    const g = this.gl;
    if (!g) return;
    g.gl.deleteBuffer(g.instances);
    g.gl.deleteBuffer(g.corners);
    g.gl.deleteVertexArray(g.vao);
    g.gl.deleteProgram(g.program);
    this.gl = null;
  }
}
