// Whether this browser has WebGL2 (checked once; the arranger falls back to its Canvas2D
// renderer without it).
let supported: boolean | null = null;

export function webgl2Supported(): boolean {
  if (supported !== null) return supported;
  if (typeof document === "undefined") return (supported = false);
  try {
    const gl = document.createElement("canvas").getContext("webgl2");
    // A real context reports its version (stub canvases, e.g. jsdom's, don't).
    const version = gl && typeof gl.getParameter === "function" ? gl.getParameter(gl.VERSION) : null;
    supported = typeof version === "string" && version.includes("WebGL 2");
    // Release the probe's context at once (browsers cap live contexts).
    gl?.getExtension("WEBGL_lose_context")?.loseContext();
  } catch {
    supported = false;
  }
  return supported;
}
