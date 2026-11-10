type FenceSample = { submittedAtMs: number, insertedAtMs?: number, lowerBoundMs: number, upperBoundMs?: number, polls: number, state: string }

export function installAblation(processor: any, video: HTMLVideoElement, mode: string) {
  const gl = processor.gl as WebGL2RenderingContext
  const originalUpload = gl.texImage2D.bind(gl)
  let imported = false
  let uploadCalls = 0
  let skippedUploads = 0
  ;(gl as any).texImage2D = (...args: any[]) => {
    if (args.at(-1) === video) {
      uploadCalls++
      if (mode.endsWith('no-upload') && imported) {
        skippedUploads++
        return
      }
      imported = true
    }
    return (originalUpload as any)(...args)
  }
  if (mode.endsWith('scissor')) {
    gl.enable(gl.SCISSOR_TEST)
    gl.scissor(0, 0, 1, 1)
  }

  const samples: FenceSample[] = []
  const supported = typeof gl.fenceSync === 'function' && typeof gl.clientWaitSync === 'function'
  let dueAt = 0
  let pending: FenceSample | undefined
  let sync: WebGLSync | null = null
  let timer: ReturnType<typeof setTimeout> | undefined
  let closed = false
  const pixelIdentities: { atMs: number, id: number, magic: number, valid: boolean }[] = []
  let pixelDueAt = 0
  const pixelAudit = new URLSearchParams(location.search).get('outputPixels') === '1'
  const cleanSync = () => {
    if (timer !== undefined) clearTimeout(timer)
    timer = undefined
    if (sync) gl.deleteSync(sync)
    sync = null
    pending = undefined
  }
  const poll = () => {
    if (closed || !pending || !sync) return
    const now = performance.now()
    pending.polls++
    const state = gl.clientWaitSync(sync, 0, 0)
    if (state === gl.ALREADY_SIGNALED || state === gl.CONDITION_SATISFIED) {
      pending.upperBoundMs = now - pending.submittedAtMs
      pending.state = 'completed'
      samples.push({ ...pending })
      cleanSync()
    }
    else if (state === gl.WAIT_FAILED || now - pending.submittedAtMs > 1000) {
      pending.state = state === gl.WAIT_FAILED ? 'failed' : 'timeout'
      samples.push({ ...pending })
      cleanSync()
    }
    else {
      pending.lowerBoundMs = now - pending.submittedAtMs
      timer = setTimeout(poll, 1)
    }
  }
  const method = mode.startsWith('sr') ? 'drawFrame' : 'renderFrame'
  const originalDraw = processor[method].bind(processor)
  processor[method] = (...args: any[]) => {
    const result = originalDraw(...args)
    if (pixelAudit && performance.now() >= pixelDueAt) {
      pixelDueAt = performance.now() + 1000
      const bytes = new Uint8Array(1024 * 4)
      gl.readPixels(0, gl.drawingBufferHeight - 33, 1024, 1, gl.RGBA, gl.UNSIGNED_BYTE, bytes)
      let id = 0, magic = 0
      for (let bit = 0; bit < 32; bit++) {
        const offset = (16 + bit * 32) * 4
        const value = Number((bytes[offset] + bytes[offset+1] + bytes[offset+2]) / 3 > 128)
        if (bit < 24) id |= value << bit
        else magic |= value << (bit - 24)
      }
      pixelIdentities.push({atMs: performance.now(), id, magic, valid: magic === 0xd3})
    }
    if (mode.endsWith('transparent') && processor.canvas.style.opacity !== '0')
      processor.canvas.style.opacity = '0'
    const now = performance.now()
    if (supported && !closed && !pending && now >= dueAt) {
      dueAt = now + 1000
      pending = { submittedAtMs: now, lowerBoundMs: 0, polls: 0, state: 'queued' }
      queueMicrotask(() => {
        if (closed || !pending) return
        sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0)
        if (!sync) { pending.state = 'unavailable'; samples.push({ ...pending }); cleanSync(); return }
        pending.insertedAtMs = performance.now()
        pending.state = 'pending'
        gl.flush()
        timer = setTimeout(poll, 1)
      })
    }
    return result
  }
  return {
    snapshot: () => ({ mode, uploadCalls, skippedUploads, pixelIdentities, scissorPixels: mode.endsWith('scissor') ? 1 : null, canvasOpacity: processor.canvas.style.opacity,
      fence: { supported, source: 'post-submit WebGL2 fence; queue and poll dispatch included', samples: samples.map(v => ({ ...v })), pending: Boolean(pending) } }),
    close: () => { closed = true; cleanSync() },
  }
}
