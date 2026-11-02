const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const root = process.cwd()
const ts = require(path.join(root, 'node_modules/typescript'))
const template = fs.readFileSync(path.join(root, 'dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs'), 'utf8')
const boundary = template.indexOf('\nasync function runCase(')
assert.ok(boundary > 0)
const shim = { exports: {} }
new Function('require', 'module', 'exports', template.slice(0, boundary)
  + '\nmodule.exports = { harness, getVideoFrameSource, vi };')(require, shim, shim.exports)
const { harness, getVideoFrameSource, vi } = shim.exports
const support = fs.readFileSync('src/player/infra/render/video-frame-gpu.test-support.ts', 'utf8').replace("import { vi } from 'vitest'", '')
const gpuProbe = new Function('vi', 'exports', ts.transpileModule(support, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText + '\nreturn exports.gpuProbe;')(vi, {})

async function run(kind, settings, diagnostics, enabled = true) {
  const h = await harness(kind, { targetFps: settings.targetFps ?? 60 }, false, 'playbackQuality')
  const source = getVideoFrameSource(h.video)
  const p = gpuProbe(h.video)
  let timerId = 0
  const timers = new Map()
  vi.stubGlobal('setTimeout', (callback, ms) => {
    const id = ++timerId
    timers.set(id, { callback, at: performance.now() + ms })
    return id
  })
  vi.stubGlobal('clearTimeout', id => timers.delete(id))
  if (!enabled) p.canvas.getContext.mockReturnValue(null)
  p.gl.fenceSync = vi.fn(() => ({ readyAt: performance.now() + (settings.delayMs ?? 1) }))
  p.gl.clientWaitSync = vi.fn(sync => performance.now() >= sync.readyAt ? p.gl.CONDITION_SATISFIED : p.gl.TIMEOUT_EXPIRED)
  const pump = until => {
    while (timers.size) {
      const [id, timer] = [...timers].sort((a, b) => a[1].at - b[1].at)[0]
      if (timer.at > until) break
      timers.delete(id)
      h.gl.spend(Math.max(0, timer.at - performance.now()))
      timer.callback()
    }
  }
  let image = 0
  const imports = []
  const candidates = []
  h.gl.texImage2D = vi.fn((...args) => { if (args.at(-1) === h.video) imports.push(image) })
  let requestId
  const subscribe = () => {
    requestId = source.request((at, frame) => {
      if (frame.newFrame) candidates.push({ at, image, localSequence: frame.localSequence, trackingSource: frame.trackingSource, rtpTimestamp: frame.rtpTimestamp })
      subscribe()
    })
  }
  subscribe()
  if (diagnostics) source.snapshotDiagnostics()
  const sourceFps = settings.sourceFps ?? 60
  let callbacks = 0
  for (let slot = 0; slot < 600; slot++) {
    const at = slot * 1000 / 60
    image = Math.floor(slot * sourceFps / 60) + 1
    p.image(settings.static ? 20 : image % 255)
    const counted = settings.perFrame ? image : Math.max(1, Math.floor(Math.floor(slot / 15) * sourceFps / 4))
    h.supply(counted)
    if (settings.pollFirst) h.raf(at)
    if ((slot + 1) % (settings.skipEvery ?? 6) !== 0) {
      callbacks++
      h.frameRaw(at + (settings.pollFirst ? 0.2 : 0), { presentedFrames: image, mediaTime: 0, rtpTimestamp: image * 3000 })
    }
    if (!settings.pollFirst) h.raf(at + 0.2)
    pump(at + 15)
  }
  const renderer = h.renderer.snapshotResources()
  const pixels = source.snapshotDiagnostics().videoPixels
  assert.equal(new Set(imports).size, imports.length, `${settings.name}: duplicate imports`)
  assert.equal(new Set(candidates.map(c => c.image)).size, candidates.length, `${settings.name}: duplicate candidates`)
  assert.equal(new Set(candidates.map(c => c.localSequence)).size, candidates.length)
  assert.equal(renderer.realtimeDrawCount, imports.length)
  assert.equal(renderer.duplicateDrawCount, 0)
  assert.equal(renderer.submitFailureCount, 0)
  assert.ok(pixels.pairs.length <= 8)
  assert.equal(pixels.readCount, pixels.completedReadCount + pixels.cancelledReadCount + pixels.pixelProbeGpuReads)
  assert.ok(candidates.filter(c => c.trackingSource === 'animationFrame').every(c => c.rtpTimestamp === undefined))
  if (enabled && !settings.static && (settings.delayMs ?? 1) <= 8) {
    if ((settings.targetFps ?? 60) >= sourceFps) assert.equal(candidates.length, sourceFps * 10, `${settings.name}: incomplete supply`)
    else assert.ok(candidates.length >= settings.targetFps * 10)
    assert.equal(imports.length, Math.min(sourceFps, settings.targetFps ?? 60) * 10, `${settings.name}: incomplete rendering`)
  }
  if (settings.static || (settings.delayMs ?? 1) > 8)
    assert.equal(pixels.deliveredFrameCount, 0)
  if (enabled) assert.equal(pixels.disabledReason, undefined)
  assert.ok(p.gl.clientWaitSync.mock.calls.every(args => args[1] === 0 && args[2] === 0))
  h.renderer.destroy()
  assert.equal(source.snapshotResources().pixelProbeGpuReads, 0)
  source.cancel(requestId)
  assert.equal(timers.size, 0)
  const closed = source.snapshotResources()
  for (const key of ['frameRequests', 'nativeFrameCallbacks', 'animationCallbacks', 'pixelProbeCanvases', 'pixelProbeGpuReads', 'pixelProbePendingPairs', 'pixelProbeTimers'])
    assert.equal(closed[key], 0, key)
  const result = { kind, name: settings.name, diagnostics, enabled, nativeCallbacks: callbacks, candidates: candidates.length, submitted: imports.length, uniqueImportedImages: new Set(imports).size, pixelDelivered: pixels.deliveredFrameCount, pixelPairs: pixels.pairCount, counterRebases: pixels.counterRebaseCount, discarded: pixels.discarded, closed }
  vi.restore()
  return result
}

;(async () => {
  const scenarios = [
    { name: 'batch250-native50', skipEvery: 6 },
    { name: 'batch250-native45', skipEvery: 4 },
    { name: 'batch250-native45-pollFirst', skipEvery: 4, pollFirst: true },
    { name: 'source30', sourceFps: 30, skipEvery: 4 },
    { name: 'cap30', targetFps: 30 },
    { name: 'static', static: true },
    { name: 'gpu3ms', skipEvery: 4, delayMs: 3 },
    { name: 'stale12ms', skipEvery: 6, delayMs: 12 },
    { name: 'perFrameCounter', perFrame: true, skipEvery: 4 },
  ]
  const results = []
  for (const kind of ['webgl2', 'webgl2_sr']) {
    for (const settings of scenarios) {
      const off = await run(kind, settings, false)
      const dev = await run(kind, settings, true)
      for (const key of ['candidates', 'submitted', 'uniqueImportedImages', 'pixelDelivered'])
        assert.equal(off[key], dev[key], `${settings.name}: dev/off mismatch ${key}`)
      results.push(off, dev)
    }
    for (const settings of scenarios.slice(0, 2)) results.push(await run(kind, settings, false, false))
  }
  const sourceFiles = ['src/player/infra/render/video-frame-source.ts', 'src/player/infra/render/video-frame-pixel-observation.ts', 'src/player/infra/render/video-frame-identity-observation.ts', 'src/player/infra/render/Renderers.ts', 'src/player/infra/render/SuperResolutionWebGL2Renderer.ts', 'src/player/infra/render/frame-pacer.ts', 'src/player/app/media/PlaybackService.ts', 'src/player/infra/render/renderer-scheduling.test.ts', 'src/player/infra/render/video-frame-source.test.ts', 'src/player/infra/render/video-frame-gpu.test-support.ts'].map(file => ({ path: file, sha256: crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex') }))
  const report = { gate: 'PASS', kind: 'production-module-GPU-async-drawable-controlled-replay', results, sourceFiles, scenarios, auditScriptSha256: crypto.createHash('sha256').update(fs.readFileSync(__filename)).digest('hex'), harnessTemplateSha256: crypto.createHash('sha256').update(template).digest('hex'), limitations: ['Video images, WebGL and clock/fence timing are controlled. Device GPU costs, actual drawable identity and screen presentation require a user-run fresh trace.', 'Unavailable-GPU results measure fallback on this same code, preserving the current native/counter supply.'] }
  const output = process.argv[2] ?? '/tmp/xbxrc-gpu-drawable-audit.metrics.json'
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify({ gate: 'PASS', cases: results.length, output, results: results.filter(r => !r.diagnostics).map(({ closed, discarded, ...r }) => r) }, null, 2))
})().catch(error => { vi.restore(); console.error(error); process.exitCode = 1 })
