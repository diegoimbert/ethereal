/**
 * A small WebGL2 wrapper: the context of one canvas, compiled programs, instanced pipelines
 * and instance batches whose GPU objects are (re)created lazily, so a lost context is
 * restored by bumping `generation` and letting everything re-upload from its CPU copy.
 */

export type GL = WebGL2RenderingContext;

/**
 * What instance batches and scenes need of a drawing context: the WebGL2 one (`GlContext`),
 * or the Canvas2D fallback's (`gl` null: batches keep their CPU copy only, see canvas2d.ts).
 */
export interface RenderContext {
  readonly gl: GL | null;
  /** Changes whenever GPU resources must be recreated (first use, context restored). */
  readonly generation: number;
}

export interface AttributeSpec {
  name: string;
  /** Floats (1–4). */
  size: number;
}

/** A compiled program with the instance attribute layout of one pipeline. */
export class Pipeline {
  program: WebGLProgram | null = null;
  readonly stride: number;
  private uniforms = new Map<string, WebGLUniformLocation | null>();
  private generation = -1;

  /**
   * `params`: constants baked into the shaders that the Canvas2D backend needs too (e.g. the
   * `fade` pipeline's clip radius).
   */
  constructor(
    readonly name: string,
    private vs: string,
    private fs: string,
    readonly attributes: ReadonlyArray<AttributeSpec>,
    readonly params: Readonly<Record<string, number>> = {},
  ) {
    this.stride = attributes.reduce((n, a) => n + a.size, 0);
  }

  /** Compiles on first use and after a context loss. */
  use(ctx: GlContext): boolean {
    const gl = ctx.gl;
    if (!gl) return false;
    if (this.generation !== ctx.generation) {
      this.program = compile(gl, this.vs, this.fs, this.name, this.attributes);
      this.uniforms.clear();
      this.generation = ctx.generation;
    }
    if (!this.program) return false;
    gl.useProgram(this.program);
    return true;
  }

  uniform(ctx: GlContext, name: string): WebGLUniformLocation | null {
    let loc = this.uniforms.get(name);
    if (loc === undefined) {
      loc = this.program && ctx.gl ? ctx.gl.getUniformLocation(this.program, name) : null;
      this.uniforms.set(name, loc);
    }
    return loc;
  }
}

function compile(gl: GL, vsSrc: string, fsSrc: string, name: string, attributes: ReadonlyArray<AttributeSpec>): WebGLProgram | null {
  const shader = (type: number, src: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS) && !gl.isContextLost()) {
      console.error(`arranger gl: ${name} shader`, gl.getShaderInfoLog(s));
    }
    return s;
  };
  const vs = shader(gl.VERTEX_SHADER, vsSrc);
  const fs = shader(gl.FRAGMENT_SHADER, fsSrc);
  const p = gl.createProgram()!;
  gl.attachShader(p, vs);
  gl.attachShader(p, fs);
  gl.bindAttribLocation(p, 0, "a_corner");
  attributes.forEach((a, i) => gl.bindAttribLocation(p, i + 1, a.name));
  gl.linkProgram(p);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
    if (!gl.isContextLost()) console.error(`arranger gl: ${name} program`, gl.getProgramInfoLog(p));
    return null;
  }
  return p;
}

/**
 * Instances of one pipeline (CPU copy + GPU buffer and VAO). `set` replaces the data; the
 * upload happens at the next draw.
 */
export class Batch {
  data: Float32Array = new Float32Array(0);
  /** Instances. */
  count = 0;
  private buffer: WebGLBuffer | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private capacity = 0;
  private generation = -1;
  private dirty = true;

  constructor(readonly pipeline: Pipeline) {}

  set(data: Float32Array, count = data.length / this.pipeline.stride): void {
    this.data = data;
    this.count = count;
    this.dirty = true;
  }

  /** Bytes on the GPU. */
  get bytes(): number {
    return this.capacity;
  }

  /** Draws instances `[first, first + n)` (all by default). The pipeline must be in use. */
  draw(ctx: GlContext, first = 0, n = this.count - first): void {
    const gl = ctx.gl;
    if (!gl || n <= 0 || this.count === 0) return;
    if (this.generation !== ctx.generation) {
      this.buffer = null;
      this.vao = null;
      this.capacity = 0;
      this.generation = ctx.generation;
      this.dirty = true;
    }
    if (!this.buffer) this.buffer = gl.createBuffer();
    if (this.dirty) {
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
      const bytes = this.count * this.pipeline.stride * 4;
      if (bytes > this.capacity) {
        this.capacity = Math.max(bytes, Math.ceil(this.capacity * 1.5));
        gl.bufferData(gl.ARRAY_BUFFER, this.capacity, gl.DYNAMIC_DRAW);
      }
      gl.bufferSubData(gl.ARRAY_BUFFER, 0, this.data, 0, this.count * this.pipeline.stride);
      this.dirty = false;
    }
    if (!this.vao) this.vao = ctx.instanceVao(this.pipeline, this.buffer!);
    gl.bindVertexArray(this.vao);
    // No base instance in WebGL2: offset the attribute pointers instead (cheap, VAO state).
    if (first !== 0) ctx.pointAttributes(this.pipeline, this.buffer!, first);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, n);
    if (first !== 0) ctx.pointAttributes(this.pipeline, this.buffer!, 0);
    gl.bindVertexArray(null);
  }

  dispose(ctx: RenderContext): void {
    const gl = ctx.gl;
    if (gl && this.generation === ctx.generation) {
      if (this.buffer) gl.deleteBuffer(this.buffer);
      if (this.vao) gl.deleteVertexArray(this.vao);
    }
    this.buffer = null;
    this.vao = null;
    this.capacity = 0;
  }
}

/**
 * The WebGL2 context of the arranger canvas. `generation` changes whenever GPU resources
 * must be recreated (first use, context restored).
 */
export class GlContext implements RenderContext {
  gl: GL | null = null;
  generation = 0;
  lost = false;
  private quad: WebGLBuffer | null = null;
  private quadGeneration = -1;
  private listeners = new Set<() => void>();
  private lostListeners = new Set<() => void>();

  constructor(readonly canvas: HTMLCanvasElement) {
    this.gl = canvas.getContext("webgl2", {
      alpha: true,
      antialias: false,
      depth: false,
      stencil: false,
      premultipliedAlpha: true,
      preserveDrawingBuffer: false,
      powerPreference: "high-performance",
    });
    canvas.addEventListener("webglcontextlost", this.onLost);
    canvas.addEventListener("webglcontextrestored", this.onRestored);
  }

  /** Called after the context was restored (GPU resources must be rebuilt: redraw). */
  onRestore(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Called when the context is lost (it may or may not come back). */
  onLose(listener: () => void): () => void {
    this.lostListeners.add(listener);
    return () => this.lostListeners.delete(listener);
  }

  private onLost = (e: Event) => {
    e.preventDefault(); // lets the browser restore it
    this.lost = true;
    for (const l of this.lostListeners) l();
  };

  private onRestored = () => {
    this.lost = false;
    this.generation++;
    for (const l of this.listeners) l();
  };

  /** Loses and restores the context (tests of the restore path). */
  simulateLoss(): boolean {
    const ext = this.gl?.getExtension("WEBGL_lose_context");
    if (!ext) return false;
    ext.loseContext();
    setTimeout(() => ext.restoreContext(), 50);
    return true;
  }

  get ready(): boolean {
    return !!this.gl && !this.lost && !this.gl.isContextLost();
  }

  /** A VAO with the unit quad (attribute 0) and `buffer`'s instance attributes. */
  instanceVao(p: Pipeline, buffer: WebGLBuffer): WebGLVertexArrayObject {
    const gl = this.gl!;
    if (this.quadGeneration !== this.generation) {
      this.quad = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
      this.quadGeneration = this.generation;
    }
    const vao = gl.createVertexArray()!;
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.quad);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    this.pointAttributes(p, buffer, 0);
    gl.bindVertexArray(null);
    return vao;
  }

  /** Instance attribute pointers of `p` into `buffer`, from instance `first` (VAO bound). */
  pointAttributes(p: Pipeline, buffer: WebGLBuffer, first: number): void {
    const gl = this.gl!;
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    let offset = 0;
    p.attributes.forEach((a, i) => {
      const loc = i + 1;
      gl.enableVertexAttribArray(loc);
      gl.vertexAttribPointer(loc, a.size, gl.FLOAT, false, p.stride * 4, (first * p.stride + offset) * 4);
      gl.vertexAttribDivisor(loc, 1);
      offset += a.size;
    });
  }

  dispose(): void {
    this.canvas.removeEventListener("webglcontextlost", this.onLost);
    this.canvas.removeEventListener("webglcontextrestored", this.onRestored);
    this.listeners.clear();
    this.lostListeners.clear();
  }
}
