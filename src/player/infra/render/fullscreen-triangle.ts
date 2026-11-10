/** 与 [-1,-1], [3,-1], [-1,3] 相同的全屏三角形，依赖 WebGL2 默认 VAO。 */
export const FULLSCREEN_VERTEX_SOURCE = `#version 300 es
void main() {
  vec2 position = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(position * 2.0 - 1.0, 0.0, 1.0);
}`

export function createFullscreenVertexShader(gl: WebGL2RenderingContext): WebGLShader {
  const shader = gl.createShader(gl.VERTEX_SHADER)
  if (!shader)
    throw new Error('webgl2VertexShaderAllocationFailed')
  gl.shaderSource(shader, FULLSCREEN_VERTEX_SOURCE)
  gl.compileShader(shader)
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const reason = gl.getShaderInfoLog(shader) ?? 'unknown'
    gl.deleteShader(shader)
    throw new Error(`webgl2VertexShaderCompileFailed:${reason}`)
  }
  return shader
}
