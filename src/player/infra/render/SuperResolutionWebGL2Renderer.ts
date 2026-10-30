/**
 * FSR1 EASU + RCAS 双 pass（WebGL2 / GLSL ES 3.00）。
 * 算法与常量布局源自 AMD GPUOpen FidelityFX-FSR `ffx_fsr1.h` / `ffx_a.h`；
 * 本文件为 API 与着色语言移植，采样回调用 `textureGather`/`texelFetch` 实现。
 *
 * 运行期合同：
 * - 当前源尺寸是所有纹理和输出的下限，等尺寸只运行 RCAS。
 * - 新帧立即尝试提交，用户显示上限使用累计节拍；资源随真实配置变化复用。
 * - context、shader、分配或绘制错误交 PlaybackService 统一保真回退。
 * - Mobile single-pass 编译成功时承担放大，双 pass 保留兼容 shader 路径。
 */

import type { RendererRuntimeConfig } from '../../domain/media'
import type { RendererResources } from './render-observation'
import type { RenderSurfaceObservation } from './render-surface-observation'
import { FrameCaptureQueue } from './frame-capture-queue'
import { FramePacer } from './frame-pacer'
import { computeFsrEasuCon, computeFsrRcasCon, rcasStopsToMobileFsrSharpness } from './fsr1-cpu'
import { FSR1_MOBILE_OPT_SINGLE_PASS_FRAGMENT } from './fsr1-mobile-opt-fragment'
import { RenderObservation } from './render-observation'
import { observeRenderSurface } from './render-surface-observation'
import { CAS_RING_BLEND, CAS_RING_PAD } from './sharpening-tuning'
import { getVideoFrameSource } from './video-frame-source'
import { readWebglDefaultFramebufferToCanvas } from './webgl-frame-capture'
import { WebglRuntimeGuard } from './webgl-runtime-guard'

const FSR_RCAS_LIMIT = (0.25 - (1.0 / 16.0))

const VERTEX_SHADER = `#version 300 es
layout(location=0) in vec4 position;
void main(){ gl_Position = position; }
`

function buildEasuFragmentSource(useCompatGather: boolean): string {
  const gatherHelpers = useCompatGather
    ? `
vec4 FsrEasuGatherComp(vec2 p, int comp){
  ivec2 size = textureSize(easuTex, 0);
  vec2 coord = p * vec2(size) - vec2(0.5);
  ivec2 base = ivec2(floor(coord));
  ivec2 i0j1 = clamp(base + ivec2(0, 1), ivec2(0), size - ivec2(1));
  ivec2 i1j1 = clamp(base + ivec2(1, 1), ivec2(0), size - ivec2(1));
  ivec2 i1j0 = clamp(base + ivec2(1, 0), ivec2(0), size - ivec2(1));
  ivec2 i0j0 = clamp(base, ivec2(0), size - ivec2(1));
  vec4 s0 = texelFetch(easuTex, i0j1, 0);
  vec4 s1 = texelFetch(easuTex, i1j1, 0);
  vec4 s2 = texelFetch(easuTex, i1j0, 0);
  vec4 s3 = texelFetch(easuTex, i0j0, 0);
  return vec4(s0[comp], s1[comp], s2[comp], s3[comp]);
}
vec4 FsrEasuRF(vec2 p){ return FsrEasuGatherComp(p, 0); }
vec4 FsrEasuGF(vec2 p){ return FsrEasuGatherComp(p, 1); }
vec4 FsrEasuBF(vec2 p){ return FsrEasuGatherComp(p, 2); }
`
    : `
vec4 FsrEasuRF(vec2 p){ return textureGather(easuTex, p, 0); }
vec4 FsrEasuGF(vec2 p){ return textureGather(easuTex, p, 1); }
vec4 FsrEasuBF(vec2 p){ return textureGather(easuTex, p, 2); }
`

  return `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D easuTex;
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
  vec4 bczzR = FsrEasuRF(p0);
  vec4 bczzG = FsrEasuGF(p0);
  vec4 bczzB = FsrEasuBF(p0);
  vec4 ijfeR = FsrEasuRF(p1);
  vec4 ijfeG = FsrEasuGF(p1);
  vec4 ijfeB = FsrEasuBF(p1);
  vec4 klhgR = FsrEasuRF(p2);
  vec4 klhgG = FsrEasuGF(p2);
  vec4 klhgB = FsrEasuBF(p2);
  vec4 zzonR = FsrEasuRF(p3);
  vec4 zzonG = FsrEasuGF(p3);
  vec4 zzonB = FsrEasuBF(p3);
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

const RCAS_FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D rcasTex;
uniform vec4 rcasCon;
uniform float brightness;
uniform float contrast;
uniform float saturation;
uniform bool applyColor;
const vec3 LUMINOSITY_FACTOR = vec3(0.299, 0.587, 0.114);
out vec4 fragColor;

float APrxMedRcpF1(float a){
  float b = uintBitsToFloat(0x7ef19fffu - floatBitsToUint(a));
  return b * (-b * a + 2.0);
}
float ASatF1(float x){ return clamp(x, 0.0, 1.0); }
float AMax3F1(float a,float b,float c){ return max(a, max(b, c)); }
float AMin3F1(float a,float b,float c){ return min(a, min(b, c)); }

vec4 FsrRcasLoadF(ivec2 p){
  return texelFetch(rcasTex, clamp(p, ivec2(0), textureSize(rcasTex, 0) - ivec2(1)), 0);
}

void FsrRcasInputF(inout float r,inout float g,inout float b){ }

void main(){
  uvec2 ip = uvec2(ivec2(gl_FragCoord.xy));
  vec4 con = rcasCon;
  ivec2 sp = ivec2(ip);
  vec3 b = FsrRcasLoadF(sp + ivec2(0, -1)).rgb;
  vec3 d = FsrRcasLoadF(sp + ivec2(-1, 0)).rgb;
  vec3 e = FsrRcasLoadF(sp).rgb;
  vec3 f = FsrRcasLoadF(sp + ivec2(1, 0)).rgb;
  vec3 h = FsrRcasLoadF(sp + ivec2(0, 1)).rgb;
  float bR=b.r,bG=b.g,bB=b.b;
  float dR=d.r,dG=d.g,dB=d.b;
  float eR=e.r,eG=e.g,eB=e.b;
  float fR=f.r,fG=f.g,fB=f.b;
  float hR=h.r,hG=h.g,hB=h.b;
  FsrRcasInputF(bR,bG,bB);
  FsrRcasInputF(dR,dG,dB);
  FsrRcasInputF(eR,eG,eB);
  FsrRcasInputF(fR,fG,fB);
  FsrRcasInputF(hR,hG,hB);
  float bL=bB*0.5+(bR*0.5+bG);
  float dL=dB*0.5+(dR*0.5+dG);
  float eL=eB*0.5+(eR*0.5+eG);
  float fL=fB*0.5+(fR*0.5+fG);
  float hL=hB*0.5+(hR*0.5+hG);
  // 噪声项：邻域 luma 方差越小（平坦噪点区）nz 越接近 0.5，把 lobe 压回去；
  // 纹理/边缘区 nz 接近 1，保持完整锐化。range 加下限避免平坦色块除零。
  float nzRange=max(AMax3F1(AMax3F1(bL,dL,eL),fL,hL)-AMin3F1(AMin3F1(bL,dL,eL),fL,hL), 1e-4);
  float nz=0.25*bL+0.25*dL+0.25*fL+0.25*hL-eL;
  nz=ASatF1(abs(nz)*APrxMedRcpF1(nzRange));
  nz=-0.5*nz+1.0;
  float mn4R=min(AMin3F1(bR,dR,fR),hR);
  float mn4G=min(AMin3F1(bG,dG,fG),hG);
  float mn4B=min(AMin3F1(bB,dB,fB),hB);
  float mx4R=max(AMax3F1(bR,dR,fR),hR);
  float mx4G=max(AMax3F1(bG,dG,fG),hG);
  float mx4B=max(AMax3F1(bB,dB,fB),hB);
  vec2 peakC = vec2(1.0, -4.0);
  float hitMinR=min(mn4R,eR)*APrxMedRcpF1(4.0*mx4R);
  float hitMinG=min(mn4G,eG)*APrxMedRcpF1(4.0*mx4G);
  float hitMinB=min(mn4B,eB)*APrxMedRcpF1(4.0*mx4B);
  float hitMaxR=(peakC.x-max(mx4R,eR))*APrxMedRcpF1(4.0*mn4R+peakC.y);
  float hitMaxG=(peakC.x-max(mx4G,eG))*APrxMedRcpF1(4.0*mn4G+peakC.y);
  float hitMaxB=(peakC.x-max(mx4B,eB))*APrxMedRcpF1(4.0*mn4B+peakC.y);
  float lobeR=max(-hitMinR,hitMaxR);
  float lobeG=max(-hitMinG,hitMaxG);
  float lobeB=max(-hitMinB,hitMaxB);
  float lobe=max(-${FSR_RCAS_LIMIT.toFixed(6)}, min(AMax3F1(lobeR,lobeG,lobeB), 0.0)) * uintBitsToFloat(floatBitsToUint(con.x)) * nz;
  float rcpL=APrxMedRcpF1(4.0*lobe+1.0);
  float pixR=(lobe*bR+lobe*dR+lobe*hR+lobe*fR+eR)*rcpL;
  float pixG=(lobe*bG+lobe*dG+lobe*hG+lobe*fG+eG)*rcpL;
  float pixB=(lobe*bB+lobe*dB+lobe*hB+lobe*fB+eB)*rcpL;
  // 邻域 ringing / 块边界控制：在 b,d,e,f,h 十字邻域 min/max 外扩一环后钳制 RCAS 输出，减轻低码率振铃与 DCT 块边界过冲。
  // 与 webgl2 CAS 路径共用同一组常量，避免两条路径观感分裂。
  const float SR_RCAS_RING_PAD_SCALE = ${CAS_RING_PAD.toFixed(4)};
  const float SR_RCAS_RING_BLEND = ${CAS_RING_BLEND.toFixed(4)};
  vec3 sharpened = vec3(pixR, pixG, pixB);
  vec3 ringLo = vec3(min(mn4R, eR), min(mn4G, eG), min(mn4B, eB));
  vec3 ringHi = vec3(max(mx4R, eR), max(mx4G, eG), max(mx4B, eB));
  vec3 ringRange = max(ringHi - ringLo, vec3(1e-4));
  vec3 ringPad = ringRange * SR_RCAS_RING_PAD_SCALE;
  vec3 ringClamped = clamp(sharpened, ringLo - ringPad, ringHi + ringPad);
  vec3 color = mix(sharpened, ringClamped, SR_RCAS_RING_BLEND);
  if (applyColor) {
    color = mix(vec3(dot(color, LUMINOSITY_FACTOR)), color, saturation / 100.0);
    color = (contrast / 100.0) * (color - 0.5) + 0.5;
    color = (brightness / 100.0) * color;
  }
  fragColor = vec4(color, 1.0);
}
`

function resolveEffectiveSrOutputSize(config: RendererRuntimeConfig, video?: HTMLVideoElement): { width: number, height: number } {
  const sourceW = video?.videoWidth || config.renderSourceWidth || 1
  const sourceH = video?.videoHeight || config.renderSourceHeight || 1
  const tierW = Math.round(config.superResolutionOutputWidth ?? sourceW)
  const tierH = Math.round(config.superResolutionOutputHeight ?? sourceH)
  const targetFitsSource = tierW >= sourceW && tierH >= sourceH
  // 标准 SR 目标由 plan 唯一确定，CSS 视口负责显示几何。
  return {
    width: targetFitsSource ? tierW : sourceW,
    height: targetFitsSource ? tierH : sourceH,
  }
}

class SuperResolutionProcessor {
  private readonly canvas: HTMLCanvasElement
  private readonly video: HTMLVideoElement
  private readonly onRuntimeDegrade?: (reason: string) => void
  private gl: WebGL2RenderingContext | null = null
  private guard?: WebglRuntimeGuard
  private uniformsDirty = true
  private uniformSourceWidth = 0
  private uniformSourceHeight = 0
  private midWidth = 0
  private midHeight = 0
  private mobileOptProgram: WebGLProgram | null = null
  private mobileCompileAttempted = false
  private surfaceVisible = false
  private mobileOptUniforms: Record<string, WebGLUniformLocation | null> = {}
  private easuProgram: WebGLProgram | null = null
  private rcasProgram: WebGLProgram | null = null
  private easuTex: WebGLTexture | null = null
  private midTex: WebGLTexture | null = null
  private midFbo: WebGLFramebuffer | null = null
  private vbo: WebGLBuffer | null = null
  private easuUniforms: Record<string, WebGLUniformLocation | null> = {}
  private rcasUniforms: Record<string, WebGLUniformLocation | null> = {}
  private videoTexAllocW = 0
  private videoTexAllocH = 0
  private runtimeDegradeEmitted = false
  private outW = 1920
  private outH = 1080
  private viewportWidthCss: number | null = null
  private viewportHeightCss: number | null = null
  private targetFps = 60
  private readonly framePacer = new FramePacer()
  private animRafId: number | null = null
  private videoFrameCbId: number | null = null
  private hasFreshVideoFrame = false
  private stopped = false
  private readonly boundDraw: () => void
  private contextListenersBound = false
  private brightness = 100.0
  private contrast = 100.0
  private saturation = 100.0
  private rcasStops = 0.88
  private readonly frameCaptures = new FrameCaptureQueue()
  private readonly drawObservation = new RenderObservation()

  snapshotResources(): RendererResources {
    return { ...this.drawObservation.snapshot(), videoUploadMethod: 'texImage2D', outputWidth: this.canvas.width, outputHeight: this.canvas.height, gpuErrorChecks: this.guard?.errorChecks, gpuObjects: [this.mobileOptProgram, this.easuProgram, this.rcasProgram, this.easuTex, this.midTex, this.midFbo, this.vbo].filter(Boolean).length, frameRequests: Number(this.videoFrameCbId !== null), animationCallbacks: Number(this.animRafId !== null), captureWaiters: this.frameCaptures.size, domNodes: Number(this.canvas.isConnected) }
  }

  snapshotDiagnostics(): RenderSurfaceObservation {
    return observeRenderSurface(this.video, this.canvas, this.gl)
  }

  private frameReadyEmitted = false
  private readonly onContextLost = (event: Event): void => {
    event.preventDefault()
    this.teardownGl(false)
    this.canvas.style.opacity = '0'
    this.emitRuntimeDegradeOnce('srContextLost')
  }

  private readonly onContextRestored = (): void => {
    if (this.stopped) {
      return
    }
    this.runtimeDegradeEmitted = false
    this.videoTexAllocW = 0
    this.videoTexAllocH = 0
    try {
      this.setupGl()
      this.startPresentationLoop()
    }
    catch (error) {
      this.emitRuntimeDegradeOnce(error instanceof Error ? error.message : 'srContextRestoreFailed')
    }
  }

  constructor(
    video: HTMLVideoElement,
    outWidth: number,
    outHeight: number,
    onRuntimeDegrade?: (reason: string) => void,
    private readonly onFrameReady?: () => void,
  ) {
    this.video = video
    this.onRuntimeDegrade = onRuntimeDegrade
    this.outW = outWidth
    this.outH = outHeight
    this.canvas = document.createElement('canvas')
    this.canvas.width = outWidth
    this.canvas.height = outHeight
    this.canvas.style.position = 'absolute'
    this.canvas.style.inset = '0'
    this.canvas.style.width = '100%'
    this.canvas.style.height = '100%'
    this.canvas.style.pointerEvents = 'none'
    video.insertAdjacentElement('afterend', this.canvas)
    this.boundDraw = this.drawFrame.bind(this)
  }

  setOutputSize(width: number, height: number): void {
    width = Math.max(Math.round(width), this.video.videoWidth || 0)
    height = Math.max(Math.round(height), this.video.videoHeight || 0)
    if (width <= 0 || height <= 0 || (width === this.outW && height === this.outH))
      return
    try {
      this.guard?.assertSize(this.video.videoWidth, this.video.videoHeight, width, height)
      this.outW = this.canvas.width = width
      this.outH = this.canvas.height = height
      this.uniformsDirty = true
      this.guard?.assertBacking(width, height)
      if (this.gl !== null) {
        this.gl.viewport(0, 0, width, height)
        this.preparePrograms(this.gl, this.video.videoWidth || width, this.video.videoHeight || height)
      }
      this.guard?.check(performance.now(), true)
    }
    catch (error) {
      this.emitRuntimeDegradeOnce(error instanceof Error ? error.message : 'srResizeFailed')
    }
  }

  setDisplayViewport(widthCss?: number, heightCss?: number): void {
    const normalizedWidthCss = widthCss !== undefined && widthCss > 0
      ? Math.round(widthCss)
      : null
    const normalizedHeightCss = heightCss !== undefined && heightCss > 0
      ? Math.round(heightCss)
      : null
    if (
      normalizedWidthCss === this.viewportWidthCss
      && normalizedHeightCss === this.viewportHeightCss
    ) {
      return
    }
    this.viewportWidthCss = normalizedWidthCss
    this.viewportHeightCss = normalizedHeightCss
    if (
      normalizedWidthCss !== null
      && normalizedHeightCss !== null
    ) {
      this.canvas.style.width = `${normalizedWidthCss}px`
      this.canvas.style.height = `${normalizedHeightCss}px`
      this.canvas.style.margin = 'auto'
      return
    }
    this.canvas.style.width = '100%'
    this.canvas.style.height = '100%'
    this.canvas.style.margin = ''
  }

  setDisplayFormat(format: RendererRuntimeConfig['format']): void {
    if (format === 'Stretch') {
      this.canvas.style.objectFit = 'fill'
    }
    else if (format === 'Zoom') {
      this.canvas.style.objectFit = 'cover'
    }
    else {
      this.canvas.style.objectFit = 'contain'
    }
  }

  setColorOptions(brightness: number, contrast: number, saturation: number): void {
    if (this.brightness !== brightness || this.contrast !== contrast || this.saturation !== saturation)
      this.uniformsDirty = true
    this.brightness = brightness
    this.contrast = contrast
    this.saturation = saturation
  }

  setRcasStops(value: number): void {
    if (!Number.isFinite(value)) {
      return
    }
    const next = Math.max(0.6, Math.min(1.1, value))
    this.uniformsDirty ||= next !== this.rcasStops
    this.rcasStops = next
  }

  setTargetFps(fps: number): void {
    if (fps === this.targetFps)
      return
    const wasBudgetActive = this.targetFps <= 0
    this.targetFps = fps
    this.framePacer.setTargetFps(fps)
    if (fps <= 0) {
      this.hasFreshVideoFrame = false
      if (this.videoFrameCbId !== null) {
        getVideoFrameSource(this.video).cancel(this.videoFrameCbId)
        this.videoFrameCbId = null
      }
      if (this.animRafId !== null && this.frameCaptures.size === 0) {
        cancelAnimationFrame(this.animRafId)
        this.animRafId = null
      }
      return
    }
    if (wasBudgetActive && !this.stopped && this.gl !== null) {
      this.scheduleVideoFrameMarker()
    }
  }

  init(): void {
    this.setupGl()
    this.startPresentationLoop()
  }

  private startPresentationLoop(): void {
    if (this.stopped) {
      return
    }
    if (this.animRafId !== null) {
      cancelAnimationFrame(this.animRafId)
      this.animRafId = null
    }
    if (this.videoFrameCbId !== null) {
      getVideoFrameSource(this.video).cancel(this.videoFrameCbId)
      this.videoFrameCbId = null
    }
    if (this.targetFps > 0) {
      this.hasFreshVideoFrame = getVideoFrameSource(this.video).lastFrame !== undefined
      this.scheduleVideoFrameMarker()
    }
    else {
      this.hasFreshVideoFrame = false
    }
    if (this.hasFreshVideoFrame)
      this.scheduleDraw()
  }

  private scheduleDraw(): void {
    if (!this.stopped && this.animRafId === null)
      this.animRafId = requestAnimationFrame(this.boundDraw)
  }

  private scheduleVideoFrameMarker(): void {
    if (this.stopped || this.targetFps <= 0) {
      return
    }
    this.videoFrameCbId = getVideoFrameSource(this.video).request((_atMs, metadata) => {
      this.videoFrameCbId = null
      if (this.hasFreshVideoFrame && metadata?.newFrame !== false)
        this.drawObservation.replaceLatest()
      this.hasFreshVideoFrame ||= metadata?.newFrame !== false
      if (!this.stopped) {
        this.scheduleVideoFrameMarker()
        if (this.hasFreshVideoFrame) {
          if (this.animRafId !== null)
            cancelAnimationFrame(this.animRafId)
          this.drawFrame()
        }
      }
    })
  }

  destroy(): void {
    this.drainFrameCaptureWaiters(false)
    this.stopped = true
    if (this.animRafId !== null) {
      cancelAnimationFrame(this.animRafId)
      this.animRafId = null
    }
    if (this.videoFrameCbId !== null) {
      getVideoFrameSource(this.video).cancel(this.videoFrameCbId)
      this.videoFrameCbId = null
    }
    if (this.canvas.isConnected) {
      this.canvas.remove()
    }
    this.teardownGl()
    this.canvas.width = 1
    this.canvas.height = 1
  }

  private teardownGl(removeListeners = true): void {
    const gl = this.gl
    if (removeListeners && this.contextListenersBound) {
      this.canvas.removeEventListener('webglcontextlost', this.onContextLost as EventListener)
      this.canvas.removeEventListener('webglcontextrestored', this.onContextRestored as EventListener)
      this.contextListenersBound = false
    }
    if (gl !== null) {
      if (this.easuProgram !== null) {
        gl.deleteProgram(this.easuProgram)
      }
      if (this.rcasProgram !== null) {
        gl.deleteProgram(this.rcasProgram)
      }
      if (this.mobileOptProgram !== null) {
        gl.deleteProgram(this.mobileOptProgram)
      }
      if (this.easuTex !== null) {
        gl.deleteTexture(this.easuTex)
      }
      if (this.midTex !== null) {
        gl.deleteTexture(this.midTex)
      }
      if (this.midFbo !== null) {
        gl.deleteFramebuffer(this.midFbo)
      }
      if (this.vbo !== null) {
        gl.deleteBuffer(this.vbo)
      }
    }
    this.gl = null
    this.guard = undefined
    this.uniformsDirty = true
    this.easuProgram = null
    this.rcasProgram = null
    this.mobileOptProgram = null
    this.mobileCompileAttempted = false
    this.mobileOptUniforms = {}
    this.easuUniforms = {}
    this.rcasUniforms = {}
    this.easuTex = null
    this.midTex = null
    this.midFbo = null
    this.vbo = null
    this.videoTexAllocW = 0
    this.videoTexAllocH = 0
  }

  private compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader {
    const sh = gl.createShader(type)!
    gl.shaderSource(sh, src)
    gl.compileShader(sh)
    if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
      const log = gl.getShaderInfoLog(sh) ?? ''
      gl.deleteShader(sh)
      throw new Error(`srShaderCompileFailed:${log}`)
    }
    return sh
  }

  private link(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
    const shaders: WebGLShader[] = []
    let program: WebGLProgram | null = null
    try {
      shaders.push(this.compile(gl, gl.VERTEX_SHADER, vs))
      shaders.push(this.compile(gl, gl.FRAGMENT_SHADER, fs))
      program = gl.createProgram()
      if (program === null)
        throw new Error('srProgramAllocationFailed')
      for (const shader of shaders)
        gl.attachShader(program, shader)
      gl.linkProgram(program)
      if (!gl.getProgramParameter(program, gl.LINK_STATUS))
        throw new Error(`srProgramLinkFailed:${gl.getProgramInfoLog(program) ?? ''}`)
      return program
    }
    catch (error) {
      if (program)
        gl.deleteProgram(program)
      throw error
    }
    finally {
      for (const shader of shaders)
        gl.deleteShader(shader)
    }
  }

  private setupGl(): void {
    const gl = this.canvas.getContext('webgl2', {
      antialias: false,
      alpha: false,
      depth: false,
      desynchronized: true,
      preserveDrawingBuffer: false,
      stencil: false,
      powerPreference: 'high-performance',
    } as WebGLContextAttributes) as WebGL2RenderingContext | null
    if (gl === null) {
      throw new Error('webgl2ContextUnavailable')
    }
    this.gl = gl
    this.canvas.style.opacity = '0'
    this.surfaceVisible = false
    if (!this.contextListenersBound) {
      this.canvas.addEventListener('webglcontextlost', this.onContextLost as EventListener)
      this.canvas.addEventListener('webglcontextrestored', this.onContextRestored as EventListener)
      this.contextListenersBound = true
    }
    this.guard = new WebglRuntimeGuard(gl)
    this.guard.assertSize(this.video.videoWidth, this.video.videoHeight, this.outW, this.outH)
    this.guard.assertBacking(this.outW, this.outH)
    // 当前context独占纹理单元0，各pass共用输出尺寸viewport。
    gl.activeTexture(gl.TEXTURE0)
    gl.viewport(0, 0, this.outW, this.outH)
    this.vbo = gl.createBuffer()
    gl.bindBuffer(gl.ARRAY_BUFFER, this.vbo)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW)
    gl.enableVertexAttribArray(0)
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0)
    this.tryInitFsrPipeline(gl)
    this.guard.check(performance.now(), true)
  }

  private tryInitFsrPipeline(gl: WebGL2RenderingContext): void {
    try {
      this.easuTex = gl.createTexture()
      if (this.easuTex === null)
        throw new Error('srTextureAllocationFailed')
      gl.bindTexture(gl.TEXTURE_2D, this.easuTex)
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
      this.preparePrograms(gl, this.video.videoWidth || this.outW, this.video.videoHeight || this.outH)
    }
    catch (error) {
      this.teardownGl()
      throw error
    }
  }

  private ensureRcasProgram(gl: WebGL2RenderingContext): void {
    if (this.rcasProgram !== null)
      return
    this.rcasProgram = this.link(gl, VERTEX_SHADER, RCAS_FRAGMENT)
    this.rcasUniforms = {
      rcasTex: gl.getUniformLocation(this.rcasProgram, 'rcasTex'),
      rcasCon: gl.getUniformLocation(this.rcasProgram, 'rcasCon'),
      brightness: gl.getUniformLocation(this.rcasProgram, 'brightness'),
      contrast: gl.getUniformLocation(this.rcasProgram, 'contrast'),
      saturation: gl.getUniformLocation(this.rcasProgram, 'saturation'),
      applyColor: gl.getUniformLocation(this.rcasProgram, 'applyColor'),
    }
    this.uniformsDirty = true
  }

  /** 等尺寸准备RCAS；实际放大才准备融合shader，兼容路径按失败结果准备。 */
  private preparePrograms(gl: WebGL2RenderingContext, vw: number, vh: number): void {
    if (this.outW === vw && this.outH === vh) {
      this.ensureRcasProgram(gl)
      return
    }
    if (this.mobileOptProgram !== null || this.easuProgram !== null)
      return
    if (!this.mobileCompileAttempted) {
      this.mobileCompileAttempted = true
      try {
        this.mobileOptProgram = this.link(gl, VERTEX_SHADER, FSR1_MOBILE_OPT_SINGLE_PASS_FRAGMENT)
      }
      catch {
        // 编译失败结算为当前context的兼容选择，后续放大复用EASU路径。
      }
      if (this.mobileOptProgram !== null) {
        this.mobileOptUniforms = {
          inputTexture: gl.getUniformLocation(this.mobileOptProgram, 'inputTexture'),
          inputTextureSize: gl.getUniformLocation(this.mobileOptProgram, 'inputTextureSize'),
          outputTextureSize: gl.getUniformLocation(this.mobileOptProgram, 'outputTextureSize'),
          uHdrToneMap: gl.getUniformLocation(this.mobileOptProgram, 'uHdrToneMap'),
          sharpness: gl.getUniformLocation(this.mobileOptProgram, 'sharpness'),
          brightness: gl.getUniformLocation(this.mobileOptProgram, 'brightness'),
          contrast: gl.getUniformLocation(this.mobileOptProgram, 'contrast'),
          saturation: gl.getUniformLocation(this.mobileOptProgram, 'saturation'),
          applyColor: gl.getUniformLocation(this.mobileOptProgram, 'applyColor'),
        }
        this.uniformsDirty = true
        return
      }
    }
    this.easuProgram = this.linkEasuProgram(gl)
    this.easuUniforms = {
      easuTex: gl.getUniformLocation(this.easuProgram, 'easuTex'),
      con0: gl.getUniformLocation(this.easuProgram, 'con0'),
      con1: gl.getUniformLocation(this.easuProgram, 'con1'),
      con2: gl.getUniformLocation(this.easuProgram, 'con2'),
      con3: gl.getUniformLocation(this.easuProgram, 'con3'),
    }
    this.ensureRcasProgram(gl)
    this.uniformsDirty = true
  }

  private linkEasuProgram(gl: WebGL2RenderingContext): WebGLProgram {
    try {
      return this.link(gl, VERTEX_SHADER, EASU_FRAGMENT)
    }
    catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (!message.includes('srShaderCompileFailed') || !message.includes('textureGather')) {
        throw error
      }
      return this.link(gl, VERTEX_SHADER, EASU_FRAGMENT_COMPAT)
    }
  }

  private releaseMidTarget(): void {
    const gl = this.gl
    if (gl === null) {
      return
    }
    this.midWidth = this.midHeight = 0
    if (this.midTex !== null) {
      gl.deleteTexture(this.midTex)
      this.midTex = null
    }
    if (this.midFbo !== null) {
      gl.deleteFramebuffer(this.midFbo)
      this.midFbo = null
    }
  }

  private resizeMidTarget(): void {
    const gl = this.gl
    if (gl === null) {
      return
    }
    if (this.midWidth === this.outW && this.midHeight === this.outH && this.midFbo !== null)
      return
    this.releaseMidTarget()
    this.midTex = gl.createTexture()
    if (this.midTex === null)
      throw new Error('srTextureAllocationFailed')
    gl.bindTexture(gl.TEXTURE_2D, this.midTex)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, this.outW, this.outH, 0, gl.RGBA, gl.UNSIGNED_BYTE, null)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    this.midFbo = gl.createFramebuffer()
    if (this.midFbo === null)
      throw new Error('srFramebufferAllocationFailed')
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.midFbo)
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.midTex, 0)
    const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER)
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    if (status !== gl.FRAMEBUFFER_COMPLETE) {
      throw new Error(`srFramebufferIncomplete:${status}`)
    }
    this.midWidth = this.outW
    this.midHeight = this.outH
    gl.bindTexture(gl.TEXTURE_2D, this.easuTex)
    this.guard?.check(performance.now(), true)
  }

  private shouldDraw(): boolean {
    return this.framePacer.admit(performance.now())
  }

  private uploadVideoSource(gl: WebGL2RenderingContext, vw: number, vh: number): void {
    gl.bindTexture(gl.TEXTURE_2D, this.easuTex!)
    // 完整video源、level0及RGB保留WebKit的GPU拷贝尝试入口。
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB, gl.RGB, gl.UNSIGNED_BYTE, this.video)
    if (this.videoTexAllocW !== vw || this.videoTexAllocH !== vh) {
      this.videoTexAllocW = vw
      this.videoTexAllocH = vh
      this.guard?.check(performance.now(), true)
    }
  }

  private syncUniforms(gl: WebGL2RenderingContext, vw: number, vh: number): void {
    if (!this.uniformsDirty && vw === this.uniformSourceWidth && vh === this.uniformSourceHeight)
      return
    this.uniformsDirty = false
    this.uniformSourceWidth = vw
    this.uniformSourceHeight = vh
    const applyColor = this.brightness !== 100 || this.contrast !== 100 || this.saturation !== 100
    const sameSize = this.outW === vw && this.outH === vh
    if (sameSize || this.mobileOptProgram === null) {
      gl.useProgram(this.rcasProgram)
      gl.uniform1i(this.rcasUniforms.rcasTex, 0)
      gl.uniform4fv(this.rcasUniforms.rcasCon, computeFsrRcasCon(this.rcasStops))
      gl.uniform1f(this.rcasUniforms.brightness, this.brightness)
      gl.uniform1f(this.rcasUniforms.contrast, this.contrast)
      gl.uniform1f(this.rcasUniforms.saturation, this.saturation)
      gl.uniform1i(this.rcasUniforms.applyColor, applyColor ? 1 : 0)
    }
    if (!sameSize && this.mobileOptProgram !== null) {
      gl.useProgram(this.mobileOptProgram)
      gl.uniform1i(this.mobileOptUniforms.inputTexture, 0)
      gl.uniform2f(this.mobileOptUniforms.inputTextureSize, vw, vh)
      gl.uniform2f(this.mobileOptUniforms.outputTextureSize, this.outW, this.outH)
      gl.uniform1f(this.mobileOptUniforms.uHdrToneMap, 0)
      gl.uniform1f(this.mobileOptUniforms.sharpness, rcasStopsToMobileFsrSharpness(this.rcasStops))
      gl.uniform1f(this.mobileOptUniforms.brightness, this.brightness)
      gl.uniform1f(this.mobileOptUniforms.contrast, this.contrast)
      gl.uniform1f(this.mobileOptUniforms.saturation, this.saturation)
      gl.uniform1i(this.mobileOptUniforms.applyColor, applyColor ? 1 : 0)
    }
    else if (!sameSize) {
      const { con0, con1, con2, con3 } = computeFsrEasuCon(vw, vh, vw, vh, this.outW, this.outH)
      gl.useProgram(this.easuProgram)
      gl.uniform1i(this.easuUniforms.easuTex, 0)
      gl.uniform4fv(this.easuUniforms.con0, con0)
      gl.uniform4fv(this.easuUniforms.con1, con1)
      gl.uniform4fv(this.easuUniforms.con2, con2)
      gl.uniform4fv(this.easuUniforms.con3, con3)
    }
  }

  private drawRcasPass(gl: WebGL2RenderingContext): void {
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    gl.useProgram(this.rcasProgram)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
  }

  private emitRuntimeDegradeOnce(reason: string): void {
    if (this.runtimeDegradeEmitted || this.onRuntimeDegrade === undefined) {
      return
    }
    this.runtimeDegradeEmitted = true
    this.onRuntimeDegrade(reason)
  }

  captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    if (this.stopped)
      return Promise.resolve(null)
    const result = this.frameCaptures.request(signal)
    if (this.frameCaptures.size > 0)
      this.scheduleDraw()
    return result
  }

  private drainFrameCaptureWaiters(presentedToDefault: boolean): void {
    if (this.frameCaptures.size === 0)
      return
    this.frameCaptures.complete(() => {
      const gl = this.gl
      return presentedToDefault && gl !== null && this.outW >= 2 && this.outH >= 2
        ? readWebglDefaultFramebufferToCanvas(gl, this.outW, this.outH)
        : null
    })
  }

  private drawFrame(): void {
    this.animRafId = null
    const startedAtMs = performance.now()
    const capture = this.frameCaptures.size > 0
    const realtime = this.targetFps > 0 && this.hasFreshVideoFrame && this.shouldDraw()
    let presentedToDefault = false
    let uploadDurationMs: number | undefined
    let submitDurationMs: number | undefined
    try {
      if (this.stopped)
        return
      if (!realtime && !capture) {
        if (this.hasFreshVideoFrame && this.targetFps > 0) {
          this.drawObservation.wait(getVideoFrameSource(this.video).lastFrame)
          this.scheduleDraw()
        }
        return
      }
      const gl = this.gl
      if (gl === null || this.easuTex === null)
        return
      const vw = this.video.videoWidth
      const vh = this.video.videoHeight
      if (vw <= 0 || vh <= 0)
        return
      this.guard!.assertSize(vw, vh, Math.max(this.outW, vw), Math.max(this.outH, vh))
      if (this.outW < vw || this.outH < vh)
        this.setOutputSize(vw, vh)
      if (this.runtimeDegradeEmitted)
        return
      this.preparePrograms(gl, vw, vh)
      const uploadStartedAtMs = performance.now()
      this.uploadVideoSource(gl, vw, vh)
      const uploadFinishedAtMs = performance.now()
      uploadDurationMs = uploadFinishedAtMs - uploadStartedAtMs
      this.syncUniforms(gl, vw, vh)
      if (this.outW === vw && this.outH === vh) {
        this.drawRcasPass(gl)
      }
      else if (this.mobileOptProgram !== null) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, null)
        gl.useProgram(this.mobileOptProgram)
        gl.drawArrays(gl.TRIANGLES, 0, 3)
      }
      else {
        this.resizeMidTarget()
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.midFbo)
        gl.useProgram(this.easuProgram)
        gl.drawArrays(gl.TRIANGLES, 0, 3)
        gl.bindTexture(gl.TEXTURE_2D, this.midTex!)
        this.drawRcasPass(gl)
      }
      this.guard!.check(startedAtMs)
      if (!this.surfaceVisible) {
        this.canvas.style.opacity = '1'
        this.surfaceVisible = true
      }
      presentedToDefault = true
      submitDurationMs = performance.now() - uploadFinishedAtMs
    }
    catch (error) {
      this.drawObservation.fail()
      this.emitRuntimeDegradeOnce(error instanceof Error ? error.message : 'srDrawFailed')
    }
    finally {
      if (presentedToDefault && !this.stopped) {
        this.drawObservation.record(getVideoFrameSource(this.video).lastFrame, !realtime, performance.now() - startedAtMs, uploadDurationMs, submitDurationMs)
        if (realtime)
          this.hasFreshVideoFrame = false
        if (!this.frameReadyEmitted) {
          this.frameReadyEmitted = true
          this.onFrameReady?.()
        }
      }
      this.drainFrameCaptureWaiters(presentedToDefault)
      if (capture && !realtime && this.hasFreshVideoFrame && this.targetFps > 0)
        this.scheduleDraw()
    }
  }
}

export class SuperResolutionWebGL2Renderer {
  readonly kind = 'webgl2_sr' as const
  private processor: SuperResolutionProcessor | null = null
  private config: RendererRuntimeConfig
  private video?: HTMLVideoElement

  constructor(config: RendererRuntimeConfig) {
    this.config = config
  }

  snapshotResources(): RendererResources | undefined {
    return this.processor?.snapshotResources()
  }

  snapshotDiagnostics(): RenderSurfaceObservation | undefined {
    return this.processor?.snapshotDiagnostics()
  }

  async attach(video: HTMLVideoElement): Promise<void> {
    this.destroy()
    this.video = video
    const { width: w, height: h } = resolveEffectiveSrOutputSize(this.config, video)
    this.processor = new SuperResolutionProcessor(
      video,
      w,
      h,
      this.config.rendererRuntimeFailureNotifier ?? this.config.superResolutionRuntimeDegradeNotifier,
      this.config.rendererFrameReadyNotifier,
    )
    this.processor.setDisplayViewport(this.config.renderViewportWidth, this.config.renderViewportHeight)
    this.processor.setDisplayFormat(this.config.format)
    this.processor.setColorOptions(this.config.brightness, this.config.contrast, this.config.saturation)
    this.processor.setRcasStops(this.config.superResolutionRcasStops ?? 0.88)
    this.processor.setTargetFps(this.config.targetFps)
    this.processor.init()
    video.dataset.renderPipeline = 'webgl2-sr'
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    const notifier = this.config.superResolutionRuntimeDegradeNotifier
    this.config = {
      ...this.config,
      ...config,
      superResolutionRuntimeDegradeNotifier:
        config.superResolutionRuntimeDegradeNotifier ?? notifier,
    }
    if (this.config.superResolutionOutputWidth !== undefined
      && this.config.superResolutionOutputHeight !== undefined) {
      const { width, height } = resolveEffectiveSrOutputSize(this.config, this.video)
      this.processor?.setOutputSize(width, height)
    }
    this.processor?.setDisplayViewport(this.config.renderViewportWidth, this.config.renderViewportHeight)
    this.processor?.setDisplayFormat(this.config.format)
    this.processor?.setColorOptions(
      this.config.brightness,
      this.config.contrast,
      this.config.saturation,
    )
    this.processor?.setRcasStops(this.config.superResolutionRcasStops ?? 0.88)
    this.processor?.setTargetFps(this.config.targetFps)
  }

  destroy(): void {
    this.processor?.destroy()
    this.processor = null
    this.video = undefined
  }

  async captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    return (await this.processor?.captureRenderedFrame(signal)) ?? null
  }
}
