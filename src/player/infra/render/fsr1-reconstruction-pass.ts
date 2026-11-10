import { computeFsrEasuCon } from './fsr1-cpu'
import { createFullscreenVertexShader } from './fullscreen-triangle'

// AMD GPUOpen FSR1 EASU 的现有 GLSL 实现，保持 textureGather 兼容入口。
export function buildEasuFragmentSource(useCompatGather: boolean): string {
  const gatherHelpers = useCompatGather
    ? `
void FsrEasuGatherRGB(vec2 p, out vec4 r, out vec4 g, out vec4 b){
  ivec2 size = textureSize(easuTex, 0);
  vec2 coord = p * vec2(size) - vec2(0.5);
  ivec2 base = ivec2(floor(coord));
  ivec2 i0j1 = clamp(base + ivec2(0, 1), ivec2(0), size - ivec2(1));
  ivec2 i1j1 = clamp(base + ivec2(1, 1), ivec2(0), size - ivec2(1));
  ivec2 i1j0 = clamp(base + ivec2(1, 0), ivec2(0), size - ivec2(1));
  ivec2 i0j0 = clamp(base, ivec2(0), size - ivec2(1));
  vec3 s0 = texelFetch(easuTex, i0j1, 0).rgb;
  vec3 s1 = texelFetch(easuTex, i1j1, 0).rgb;
  vec3 s2 = texelFetch(easuTex, i1j0, 0).rgb;
  vec3 s3 = texelFetch(easuTex, i0j0, 0).rgb;
  r = vec4(s0.r, s1.r, s2.r, s3.r);
  g = vec4(s0.g, s1.g, s2.g, s3.g);
  b = vec4(s0.b, s1.b, s2.b, s3.b);
}
`
    : `
void FsrEasuGatherRGB(vec2 p, out vec4 r, out vec4 g, out vec4 b){
  r = textureGather(easuTex, p, 0);
  g = textureGather(easuTex, p, 1);
  b = textureGather(easuTex, p, 2);
}
`
  // 每组位置只取一次RGB，保留gather通道顺序与EASU的12个有效采样点。
  const gatherSamples = ['bczz', 'ijfe', 'klhg', 'zzon'].map((name, index) => `
  vec4 ${name}R, ${name}G, ${name}B;
  FsrEasuGatherRGB(p${index}, ${name}R, ${name}G, ${name}B);`).join('')

  return `#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D easuTex;
uniform vec4 con0;
uniform vec4 con1;
uniform vec4 con2;
uniform vec4 con3;
out vec4 fragColor;

vec2 AF2_AU2(uvec2 v){ return uintBitsToFloat(v); }
float APrxLoRsqF1(float a){ return uintBitsToFloat(0x5f347d74u - (floatBitsToUint(a) >> 1u)); }
float APrxLoRcpF1(float a){ return uintBitsToFloat(0x7ef07ebbu - floatBitsToUint(a)); }
float APrxMedRcpF1(float a){
  float b = uintBitsToFloat(0x7ef19fffu - floatBitsToUint(a));
  return b * (-b * a + 2.0);
}
float ARcpF1(float a){ return 1.0 / a; }
float ASatF1(float x){ return clamp(x, 0.0, 1.0); }
float AMax3F1(float a,float b,float c){ return max(a, max(b, c)); }
float AMin3F1(float a,float b,float c){ return min(a, min(b, c)); }
vec3 AMax3F3(vec3 a,vec3 b,vec3 c){ return max(a, max(b, c)); }
vec3 AMin3F3(vec3 a,vec3 b,vec3 c){ return min(a, min(b, c)); }

${gatherHelpers}

void FsrEasuTapF(inout vec3 aC, inout float aW, vec2 off, vec2 dir, vec2 len, float lob, float clp, vec3 c){
  vec2 v;
  v.x = (off.x * dir.x) + (off.y * dir.y);
  v.y = (off.x * (-dir.y)) + (off.y * dir.x);
  v *= len;
  float d2 = v.x * v.x + v.y * v.y;
  d2 = min(d2, clp);
  float wB = (2.0 / 5.0) * d2 + (-1.0);
  float wA = lob * d2 + (-1.0);
  wB *= wB;
  wA *= wA;
  wB = (25.0 / 16.0) * wB + (-(25.0 / 16.0 - 1.0));
  float w = wB * wA;
  aC += c * w;
  aW += w;
}

void FsrEasuSetF(inout vec2 dir, inout float len, vec2 pp, bool biS, bool biT, bool biU, bool biV,
  float lA,float lB,float lC,float lD,float lE){
  float w = 0.0;
  if(biS) w = (1.0 - pp.x) * (1.0 - pp.y);
  if(biT) w = pp.x * (1.0 - pp.y);
  if(biU) w = (1.0 - pp.x) * pp.y;
  if(biV) w = pp.x * pp.y;
  float dc = lD - lC;
  float cb = lC - lB;
  float lenX = max(abs(dc), abs(cb));
  lenX = APrxLoRcpF1(lenX);
  float dirX = lD - lB;
  dir.x += dirX * w;
  lenX = ASatF1(abs(dirX) * lenX);
  lenX *= lenX;
  len += lenX * w;
  float ec = lE - lC;
  float ca = lC - lA;
  float lenY = max(abs(ec), abs(ca));
  lenY = APrxLoRcpF1(lenY);
  float dirY = lE - lA;
  dir.y += dirY * w;
  lenY = ASatF1(abs(dirY) * lenY);
  lenY *= lenY;
  len += lenY * w;
}

void main(){
  uvec2 ip = uvec2(ivec2(gl_FragCoord.xy));
  vec2 pp = vec2(ip) * AF2_AU2(uvec2(floatBitsToUint(con0.x), floatBitsToUint(con0.y)))
    + AF2_AU2(uvec2(floatBitsToUint(con0.z), floatBitsToUint(con0.w)));
  vec2 fp = floor(pp);
  pp -= fp;
  vec2 p0 = fp * AF2_AU2(uvec2(floatBitsToUint(con1.x), floatBitsToUint(con1.y)))
    + AF2_AU2(uvec2(floatBitsToUint(con1.z), floatBitsToUint(con1.w)));
  vec2 p1 = p0 + AF2_AU2(uvec2(floatBitsToUint(con2.x), floatBitsToUint(con2.y)));
  vec2 p2 = p0 + AF2_AU2(uvec2(floatBitsToUint(con2.z), floatBitsToUint(con2.w)));
  vec2 p3 = p0 + AF2_AU2(uvec2(floatBitsToUint(con3.x), floatBitsToUint(con3.y)));
${gatherSamples}
  vec4 bczzL = bczzB * 0.5 + (bczzR * 0.5 + bczzG);
  vec4 ijfeL = ijfeB * 0.5 + (ijfeR * 0.5 + ijfeG);
  vec4 klhgL = klhgB * 0.5 + (klhgR * 0.5 + klhgG);
  vec4 zzonL = zzonB * 0.5 + (zzonR * 0.5 + zzonG);
  float bL = bczzL.x;
  float cL = bczzL.y;
  float iL = ijfeL.x;
  float jL = ijfeL.y;
  float fL = ijfeL.z;
  float eL = ijfeL.w;
  float kL = klhgL.x;
  float lL = klhgL.y;
  float hL = klhgL.z;
  float gL = klhgL.w;
  float oL = zzonL.z;
  float nL = zzonL.w;
  vec2 dir = vec2(0.0);
  float len = 0.0;
  FsrEasuSetF(dir, len, pp, true, false, false, false, bL, eL, fL, gL, jL);
  FsrEasuSetF(dir, len, pp, false, true, false, false, cL, fL, gL, hL, kL);
  FsrEasuSetF(dir, len, pp, false, false, true, false, fL, iL, jL, kL, nL);
  FsrEasuSetF(dir, len, pp, false, false, false, true, gL, jL, kL, lL, oL);
  vec2 dir2 = dir * dir;
  float dirR = dir2.x + dir2.y;
  bool zro = dirR < (1.0 / 32768.0);
  dirR = APrxLoRsqF1(dirR);
  dirR = zro ? 1.0 : dirR;
  dir.x = zro ? 1.0 : dir.x;
  dir *= dirR;
  len = len * 0.5;
  len *= len;
  float stretch = (dir.x * dir.x + dir.y * dir.y) * APrxLoRcpF1(max(abs(dir.x), abs(dir.y)));
  vec2 len2 = vec2(1.0 + (stretch - 1.0) * len, 1.0 + (-0.5) * len);
  float lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
  float clp = APrxLoRcpF1(lob);
  vec3 min4 = min(AMin3F3(vec3(ijfeR.z, ijfeG.z, ijfeB.z), vec3(klhgR.w, klhgG.w, klhgB.w), vec3(ijfeR.y, ijfeG.y, ijfeB.y)),
    vec3(klhgR.x, klhgG.x, klhgB.x));
  vec3 max4 = max(AMax3F3(vec3(ijfeR.z, ijfeG.z, ijfeB.z), vec3(klhgR.w, klhgG.w, klhgB.w), vec3(ijfeR.y, ijfeG.y, ijfeB.y)),
    vec3(klhgR.x, klhgG.x, klhgB.x));
  vec3 aC = vec3(0.0);
  float aW = 0.0;
  FsrEasuTapF(aC, aW, vec2(0.0, -1.0) - pp, dir, len2, lob, clp, vec3(bczzR.x, bczzG.x, bczzB.x));
  FsrEasuTapF(aC, aW, vec2(1.0, -1.0) - pp, dir, len2, lob, clp, vec3(bczzR.y, bczzG.y, bczzB.y));
  FsrEasuTapF(aC, aW, vec2(-1.0, 1.0) - pp, dir, len2, lob, clp, vec3(ijfeR.x, ijfeG.x, ijfeB.x));
  FsrEasuTapF(aC, aW, vec2(0.0, 1.0) - pp, dir, len2, lob, clp, vec3(ijfeR.y, ijfeG.y, ijfeB.y));
  FsrEasuTapF(aC, aW, vec2(0.0, 0.0) - pp, dir, len2, lob, clp, vec3(ijfeR.z, ijfeG.z, ijfeB.z));
  FsrEasuTapF(aC, aW, vec2(-1.0, 0.0) - pp, dir, len2, lob, clp, vec3(ijfeR.w, ijfeG.w, ijfeB.w));
  FsrEasuTapF(aC, aW, vec2(1.0, 1.0) - pp, dir, len2, lob, clp, vec3(klhgR.x, klhgG.x, klhgB.x));
  FsrEasuTapF(aC, aW, vec2(2.0, 1.0) - pp, dir, len2, lob, clp, vec3(klhgR.y, klhgG.y, klhgB.y));
  FsrEasuTapF(aC, aW, vec2(2.0, 0.0) - pp, dir, len2, lob, clp, vec3(klhgR.z, klhgG.z, klhgB.z));
  FsrEasuTapF(aC, aW, vec2(1.0, 0.0) - pp, dir, len2, lob, clp, vec3(klhgR.w, klhgG.w, klhgB.w));
  FsrEasuTapF(aC, aW, vec2(1.0, 2.0) - pp, dir, len2, lob, clp, vec3(zzonR.z, zzonG.z, zzonB.z));
  FsrEasuTapF(aC, aW, vec2(0.0, 2.0) - pp, dir, len2, lob, clp, vec3(zzonR.w, zzonG.w, zzonB.w));
  vec3 pix = min(max4, max(min4, aC * vec3(ARcpF1(aW))));
  fragColor = vec4(pix, 1.0);
}
`
}

/** 与 `ffx_fsr1.h` 中 `FSR_EASU_F` 路径等价的单像素 EASU（32-bit） */
const EASU_FRAGMENT = buildEasuFragmentSource(false)
const EASU_FRAGMENT_COMPAT = buildEasuFragmentSource(true)

/** FSR 重建是同一 WebGL 引擎内的可选 pass，锐化由最终处理 shader 统一执行。 */
export class Fsr1ReconstructionPass {
  private program: WebGLProgram | null = null
  private readonly shaders: WebGLShader[] = []
  private texture: WebGLTexture | null = null
  private framebuffer: WebGLFramebuffer | null = null
  private width = 0
  private height = 0
  private uniforms: (WebGLUniformLocation | null)[] = []
  private sourceWidth = 0
  private sourceHeight = 0

  constructor(private readonly gl: WebGL2RenderingContext) {}

  get gpuObjects(): number {
    return [this.program, this.texture, this.framebuffer, ...this.shaders].filter(Boolean).length
  }

  private prepare(): void {
    if (this.program)
      return
    const gl = this.gl
    let shader: WebGLShader
    try {
      shader = this.compile(EASU_FRAGMENT)
    }
    catch (error) {
      if (!(error instanceof Error) || !error.message.includes('textureGather'))
        throw error
      shader = this.compile(EASU_FRAGMENT_COMPAT)
    }
    const vertex = createFullscreenVertexShader(gl)
    this.shaders.push(vertex)
    const program = gl.createProgram()
    if (!program)
      throw new Error('srProgramAllocationFailed')
    this.program = program
    gl.attachShader(program, vertex)
    gl.attachShader(program, shader)
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error(`srProgramLinkFailed:${gl.getProgramInfoLog(program) ?? 'unknown'}`)
    for (const linked of this.shaders.splice(0)) {
      gl.detachShader?.(program, linked)
      gl.deleteShader(linked)
    }
    this.uniforms = ['con0', 'con1', 'con2', 'con3'].map(name => gl.getUniformLocation(program, name))
    gl.useProgram(program)
    gl.uniform1i(gl.getUniformLocation(program, 'easuTex'), 0)
  }

  private compile(source: string): WebGLShader {
    const gl = this.gl
    const shader = gl.createShader(gl.FRAGMENT_SHADER)
    if (!shader)
      throw new Error('srShaderAllocationFailed')
    gl.shaderSource(shader, source)
    gl.compileShader(shader)
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      const message = gl.getShaderInfoLog(shader) ?? 'unknown'
      gl.deleteShader(shader)
      throw new Error(`srShaderCompileFailed:${message}`)
    }
    this.shaders.push(shader)
    return shader
  }

  render(input: WebGLTexture, sourceWidth: number, sourceHeight: number, width: number, height: number): WebGLTexture {
    this.prepare()
    const gl = this.gl
    const resized = width !== this.width || height !== this.height
    if (resized) {
      this.releaseTarget()
      this.texture = gl.createTexture()
      this.framebuffer = gl.createFramebuffer()
      if (!this.texture || !this.framebuffer)
        throw new Error('srTargetAllocationFailed')
      gl.bindTexture(gl.TEXTURE_2D, this.texture)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null)
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer)
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.texture, 0)
      const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER)
      gl.bindFramebuffer(gl.FRAMEBUFFER, null)
      if (status !== gl.FRAMEBUFFER_COMPLETE)
        throw new Error(`srFramebufferIncomplete:${status}`)
      this.width = width
      this.height = height
    }
    gl.useProgram(this.program)
    if (resized || sourceWidth !== this.sourceWidth || sourceHeight !== this.sourceHeight) {
      const constants = computeFsrEasuCon(sourceWidth, sourceHeight, sourceWidth, sourceHeight, width, height)
      ;[constants.con0, constants.con1, constants.con2, constants.con3].forEach((value, index) => gl.uniform4fv(this.uniforms[index]!, value))
      this.sourceWidth = sourceWidth
      this.sourceHeight = sourceHeight
    }
    gl.bindTexture(gl.TEXTURE_2D, input)
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    return this.texture!
  }

  private releaseTarget(): void {
    const gl = this.gl
    if (this.texture)
      gl.deleteTexture(this.texture)
    if (this.framebuffer)
      gl.deleteFramebuffer(this.framebuffer)
    this.texture = this.framebuffer = null
    this.width = this.height = 0
  }

  dispose(): void {
    this.releaseTarget()
    if (this.program)
      this.gl.deleteProgram(this.program)
    for (const shader of this.shaders) this.gl.deleteShader(shader)
    this.shaders.length = 0
    this.program = null
  }
}
