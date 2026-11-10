type CaptureContext = { nativeSerial: number, localSequence?: number, nativePresentedFrames?: number }
type Sample = CaptureContext & { atMs: number, completedAtMs?: number, id?: number, magic?: number, valid?: boolean, state: string }

// 测试源把 24 位帧号和 8 位校验标记画入图像；PBO/fence 只识别此本地合成源。
export function createDrawableIdentitySampler(video: HTMLVideoElement) {
  const canvas = document.createElement('canvas')
  canvas.width = 32
  canvas.height = 1
  const gl = canvas.getContext('webgl2', { alpha: false, antialias: false, depth: false, stencil: false })!
  if (!gl) throw new Error('Drawable marker WebGL2 unavailable')
  const compile = (kind: number, source: string) => {
    const shader = gl.createShader(kind)!
    gl.shaderSource(shader, source)
    gl.compileShader(shader)
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) ?? 'marker shader error')
    return shader
  }
  const vertex = compile(gl.VERTEX_SHADER, `#version 300 es
  void main() { vec2 p=vec2(float((gl_VertexID<<1)&2),float(gl_VertexID&2)); gl_Position=vec4(p*2.0-1.0,0.0,1.0); }`)
  const fragment = compile(gl.FRAGMENT_SHADER, `#version 300 es
  precision highp float;
  uniform sampler2D source;
  out vec4 color;
  void main() { color=texture(source,vec2((16.0+floor(gl_FragCoord.x)*32.0)/2560.0,32.0/1440.0)); }`)
  const program = gl.createProgram()!
  gl.attachShader(program, vertex)
  gl.attachShader(program, fragment)
  gl.linkProgram(program)
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? 'marker link error')
  gl.useProgram(program)
  const texture = gl.createTexture()!
  gl.bindTexture(gl.TEXTURE_2D, texture)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
  const slots = Array.from({ length: 3 }, () => {
    const buffer = gl.createBuffer()!
    gl.bindBuffer(gl.PIXEL_PACK_BUFFER, buffer)
    gl.bufferData(gl.PIXEL_PACK_BUFFER, 128, gl.STREAM_READ)
    return { buffer, sync: null as WebGLSync | null, sample: undefined as Sample | undefined }
  })
  gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null)
  const samples: Sample[] = []
  const cpuCosts: { atMs: number, durationMs: number }[] = []
  let stopped = false
  let busySkips = 0
  const poll = () => {
    for (const slot of slots) {
      if (!slot.sync || !slot.sample) continue
      const status = gl.clientWaitSync(slot.sync, 0, 0)
      if (status === gl.TIMEOUT_EXPIRED && performance.now() - slot.sample.atMs < 500) continue
      const sample = slot.sample
      sample.completedAtMs = performance.now()
      if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED) {
        const bytes = new Uint8Array(128)
        gl.bindBuffer(gl.PIXEL_PACK_BUFFER, slot.buffer)
        gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, bytes)
        gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null)
        let id = 0, magic = 0
        for (let bit = 0; bit < 32; bit++) {
          const value = Number((bytes[bit*4] + bytes[bit*4+1] + bytes[bit*4+2]) / 3 > 128)
          if (bit < 24) id |= value << bit
          else magic |= value << (bit - 24)
        }
        Object.assign(sample, { id, magic, valid: magic === 0xd3, state: 'completed' })
      }
      else sample.state = status === gl.WAIT_FAILED ? 'waitFailed' : 'expired'
      samples.push({ ...sample })
      gl.deleteSync(slot.sync)
      slot.sync = null
      slot.sample = undefined
    }
  }
  return {
    sample: (context: CaptureContext) => {
      if (stopped) return
      const atMs = performance.now()
      poll()
      const slot = slots.find(value => !value.sync)
      if (!slot) { busySkips++; return }
      gl.bindTexture(gl.TEXTURE_2D, texture)
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, video)
      gl.drawArrays(gl.TRIANGLES, 0, 3)
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, slot.buffer)
      gl.readPixels(0, 0, 32, 1, gl.RGBA, gl.UNSIGNED_BYTE, 0)
      gl.bindBuffer(gl.PIXEL_PACK_BUFFER, null)
      slot.sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0)
      slot.sample = { ...context, atMs, state: 'pending' }
      gl.flush()
      cpuCosts.push({ atMs, durationMs: performance.now() - atMs })
    },
    finish: async () => {
      stopped = true
      const deadline = performance.now() + 550
      while (slots.some(slot => slot.sync) && performance.now() < deadline) {
        poll()
        await new Promise(resolve => setTimeout(resolve, 4))
      }
    },
    snapshot: (startAtMs: number, endAtMs: number) => {
      const window = samples.filter(sample => sample.atMs >= startAtMs && sample.atMs <= endAtMs).sort((a,b) => a.atMs-b.atMs)
      const valid = window.filter(sample => sample.valid)
      const changed: Sample[] = [], extra: Sample[] = [], unqualified: Sample[] = []
      let duplicateCount = 0, skippedSourceIds = 0
      for (let i=1;i<valid.length;i++) {
        const before=valid[i-1], after=valid[i]
        if (before.id === after.id) { duplicateCount++; continue }
        changed.push(after)
        if (after.id! > before.id!) skippedSourceIds += after.id!-before.id!-1
        if (before.nativeSerial === after.nativeSerial) {
          extra.push(after)
          if (before.localSequence === after.localSequence) unqualified.push(after)
        }
      }
      const cost = cpuCosts.filter(value => value.atMs >= startAtMs && value.atMs <= endAtMs).map(value => value.durationMs).sort((a,b) => a-b)
      return { source: '32-pixel synthetic frame barcode; asynchronous PBO readback', busySkips,
        sampleCount: window.length, validCount: valid.length, uniqueFrameIds: new Set(valid.map(value=>value.id)).size,
        changedCount: changed.length, drawableChangeFps: changed.length*1000/(endAtMs-startAtMs), duplicateCount, skippedSourceIds,
        newImageWithoutNativeCallbackCount: extra.length, newImageWithoutSourceQualificationCount: unqualified.length,
        cpuP95Ms: cost[Math.max(0,Math.ceil(cost.length*.95)-1)] ?? null, cpuMaxMs: cost.at(-1) ?? null,
        extraExamples: extra.slice(0,12), samples: window }
    },
    close: () => {
      stopped = true
      for (const slot of slots) { if (slot.sync) gl.deleteSync(slot.sync); gl.deleteBuffer(slot.buffer) }
      gl.deleteTexture(texture)
      gl.deleteProgram(program)
      gl.deleteShader(vertex)
      gl.deleteShader(fragment)
      gl.getExtension('WEBGL_lose_context')?.loseContext()
      canvas.width = canvas.height = 1
    },
  }
}
