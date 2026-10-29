const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
// 从仓库根目录运行；生产模块保持原样，DOM/WebGL 与输入节拍采用受控替身。
const root = process.cwd()
const outputPath = process.argv[2] ?? '/tmp/xbxrc-sr-offline-audit.json'
const ts = require(path.join(root, 'node_modules/typescript'))
const cache = new Map()
function load(file) {
  if (!path.extname(file)) file += '.ts'
  if (cache.has(file)) return cache.get(file).exports
  const module = { exports: {} }
  cache.set(file, module)
  const source = ts.transpileModule(fs.readFileSync(file, 'utf8'), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  new Function('require', 'exports', 'module', source)(
    spec => spec.startsWith('.') ? load(path.resolve(path.dirname(file), spec)) : require(spec),
    module.exports, module,
  )
  return module.exports
}
const restorers = []
const vi = {
  fn(implementation = () => undefined) {
    const fn = (...args) => { fn.mock.calls.push(args); return implementation(...args) }
    fn.mock = { calls: [] }
    fn.mockReturnValue = value => { implementation = () => value; return fn }
    return fn
  },
  spyOn(object, key) {
    const descriptor = Object.getOwnPropertyDescriptor(object, key)
    restorers.push(() => descriptor ? Object.defineProperty(object, key, descriptor) : delete object[key])
    return { mockImplementation(fn) { Object.defineProperty(object, key, { configurable: true, value: fn }); return this } }
  },
  stubGlobal(key, value) {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, key)
    restorers.push(() => descriptor ? Object.defineProperty(globalThis, key, descriptor) : delete globalThis[key])
    Object.defineProperty(globalThis, key, { configurable: true, writable: true, value })
  },
  restore() { while (restorers.length) restorers.pop()() },
}
const rendererPath = path.join(root, 'src/player/infra/render')
const { SuperResolutionWebGL2Renderer } = load(path.join(rendererPath, 'SuperResolutionWebGL2Renderer.ts'))
const { WebGL2VideoRenderer } = load(path.join(rendererPath, 'Renderers.ts'))
const { getVideoFrameSource } = load(path.join(rendererPath, 'video-frame-source.ts'))
const testSource = fs.readFileSync(path.join(rendererPath, 'renderer-scheduling.test.ts'), 'utf8')
const selected = testSource.slice(testSource.indexOf('const config:'), testSource.indexOf('\nafterEach('))
  .replace('return { renderer, gl, video, canvases, failure, rafs, frames, frame, raf, run, supply, runSparse }',
    `const frameRaw = (at, metadata) => {
      atMs = at
      for (const [id, callback] of [...frames]) {
        frames.delete(id)
        callback(at, metadata)
      }
    }
    return { renderer, gl, video, canvases, failure, rafs, frames, frame, frameRaw, raf, run, supply, runSparse }`)
const harness = new Function('vi', 'SuperResolutionWebGL2Renderer', 'WebGL2VideoRenderer',
  ts.transpileModule(selected + '\nexports.harness = harness', {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText.replace('exports.harness = harness;', 'return harness;'))(vi, SuperResolutionWebGL2Renderer, WebGL2VideoRenderer)

async function runCase(name, { nativeMode = 'perFrame', counterMode = 'perFrame', mediaMode = 'zero', batchMs = 250, refresh = 60, nativeFirst = true } = {}) {
  const h = await harness('webgl2_sr', {}, false, 'playbackQuality')
  const source = getVideoFrameSource(h.video)
  const observations = []
  const steps = []
  const state = () => ({ nativeBaseline: source.lastPresentedFrames, counter: source.counter?.frames, uncountedNativeFrames: source.uncountedNativeFrames, counterFramesAheadOfNative: source.counterFramesAheadOfNative })
  const observe = source.observe
  source.observe = function (at, tracking, metadata) {
    const before = state()
    const previous = this.frameSequence
    observe.call(this, at, tracking, metadata)
    if (steps.length < 28) steps.push({ at: Math.round(at), tracking, presentedFrames: metadata?.presentedFrames, before, after: state(), delivered: this.frameSequence > previous, fresh: this.lastFrame?.localSequence === this.frameSequence })
  }
  const observeNext = () => source.request((at, metadata) => { observations.push({ at, ...metadata }); observeNext() })
  observeNext()
  let sourceAt = 0, displayAt = 0, sequence = 0, displaySlot = 0
  let nativeBatchCounter = 0
  const durationMs = 10000
  const counterValue = at => counterMode === 'perFrame' ? sequence : counterMode === 'fixed' ? 0 : counterMode === 'nativeBatch' ? nativeBatchCounter : Math.floor(Math.floor(at / batchMs) * batchMs * 30 / 1000)
  while (Math.min(sourceAt, displayAt) < durationMs - 0.001) {
    const isSource = nativeFirst ? sourceAt <= displayAt + 0.001 : sourceAt < displayAt - 0.001
    if (isSource) {
      const at = sourceAt
      sequence++
      nativeBatchCounter = Math.floor(Math.floor(at / batchMs) * batchMs * 30 / 1000)
      h.supply(counterValue(at))
      const nativeFrames = nativeMode === 'perFrame' ? sequence : nativeMode === 'fixed' ? 0 : Math.floor(Math.floor(at / batchMs) * batchMs * 30 / 1000)
      const metadata = { mediaTime: mediaMode === 'zero' ? 0 : sequence / 30 }
      if (nativeMode !== 'missing') metadata.presentedFrames = nativeFrames
      h.frameRaw(at, metadata)
      sourceAt = sequence * 1000 / 30
    }
    else {
      h.supply(counterValue(displayAt))
      h.raf(displayAt)
      displayAt = ++displaySlot * 1000 / refresh
    }
  }
  const fresh = observations.filter(item => item.newFrame)
  const resources = h.renderer.snapshotResources()
  assert.equal(h.gl.drawArrays.mock.calls.length, resources.realtimeDrawCount + resources.captureDrawCount)
  assert.equal(resources.submitFailureCount, 0)
  const result = {
    name, durationMs, sourceFrames: sequence,
    nativeCallbacks: source.snapshotResources().nativeCallbackCount,
    polledFrames: source.snapshotResources().polledFrameCount,
    acceptedMarkers: fresh.length,
    advancement: fresh.reduce((sum, item) => sum + (item.presentedFramesDelta ?? 1), 0),
    realtimeDrawCount: resources.realtimeDrawCount,
    realtimeDrawFps: resources.realtimeDrawCount / 10,
    gpuDrawCalls: h.gl.drawArrays.mock.calls.length,
    pacingWaitCount: resources.pacingWaitCount,
    latestOverwriteCount: resources.latestOverwriteCount,
    firstAcceptedAtMs: fresh.slice(0, 12).map(item => Math.round(item.at)),
    steps,
  }
  h.renderer.destroy()
  for (const id of [...source.pending.keys()]) source.cancel(id)
  vi.restore()
  return result
}

async function auditDrawPath(name, options, dualPass, expectedPasses) {
  const h = await harness('webgl2_sr', { renderSourceWidth: 32, renderSourceHeight: 18, ...options }, dualPass)
  for (let i = 0; i < 30; i++) h.frame(i * 1000 / 30, i + 1, i / 30)
  assert.equal(h.renderer.snapshotResources().realtimeDrawCount, 30)
  assert.equal(h.gl.drawArrays.mock.calls.length, 30 * expectedPasses)
  const realtimeGpuCalls = h.gl.drawArrays.mock.calls.length
  h.renderer.update({ targetFps: 0 })
  const capture = h.renderer.captureRenderedFrame()
  h.raf(1100)
  assert.ok(await capture)
  const resources = h.renderer.snapshotResources()
  assert.equal(resources.realtimeDrawCount, 30)
  assert.equal(resources.captureDrawCount, 1)
  assert.equal(resources.drawCount, 31)
  assert.equal(h.gl.drawArrays.mock.calls.length, 31 * expectedPasses)
  const result = { name, expectedPasses, realtimeGpuCalls, gpuCallsIncludingCapture: h.gl.drawArrays.mock.calls.length, resources, canvasStyle: h.canvases[0].style, insertion: h.video.insertAdjacentElement.mock.calls[0]?.[0] }
  h.renderer.destroy()
  vi.restore()
  return result
}

async function auditConcurrentCapture() {
  const h = await harness('webgl2_sr', { renderSourceWidth: 32, renderSourceHeight: 18 })
  const capture = h.renderer.captureRenderedFrame()
  h.frame(0, 1)
  assert.ok(await capture)
  const resources = h.renderer.snapshotResources()
  assert.equal(resources.drawCount, 1)
  assert.equal(resources.realtimeDrawCount, 1)
  assert.equal(resources.captureDrawCount, 0)
  assert.equal(h.gl.drawArrays.mock.calls.length, 1)
  const result = { name: 'capture concurrent with a fresh realtime frame', gpuCalls: h.gl.drawArrays.mock.calls.length, resources }
  h.renderer.destroy()
  vi.restore()
  return result
}

function auditCallSites() {
  const file = path.join(rendererPath, 'SuperResolutionWebGL2Renderer.ts')
  const source = ts.createSourceFile(file, fs.readFileSync(file, 'utf8'), ts.ScriptTarget.ES2022, true)
  const sites = []
  function visit(node) {
    if (ts.isCallExpression(node) && ts.isPropertyAccessExpression(node.expression)
        && ['drawArrays', 'drawElements', 'drawRcasPass', 'record'].includes(node.expression.name.text)) {
      let method = node.parent
      while (method && !ts.isMethodDeclaration(method)) method = method.parent
      sites.push({ line: source.getLineAndCharacterOfPosition(node.getStart()).line + 1, call: node.expression.getText(source), method: method?.name?.getText(source) })
    }
    ts.forEachChild(node, visit)
  }
  visit(source)
  assert.equal(sites.filter(item => item.call.endsWith('.drawArrays')).length, 3)
  assert.equal(sites.filter(item => item.call.endsWith('.record')).length, 1)
  return sites
}

;(async () => {
  const cases = []
  cases.push(await runCase('30fps native + per-frame counter, mediaTime=0'))
  cases.push(await runCase('30fps native + 250ms batch counter, mediaTime=0', { counterMode: 'batch' }))
  cases.push(await runCase('30fps native + 250ms batch counter, poll first', { counterMode: 'batch', nativeFirst: false }))
  cases.push(await runCase('250ms batch native + batch counter, mediaTime=0', { nativeMode: 'batch', counterMode: 'batch' }))
  cases.push(await runCase('275ms batch native + batch counter, mediaTime=0', { nativeMode: 'batch', counterMode: 'batch', batchMs: 275 }))
  cases.push(await runCase('275ms batch native + counter refreshed during native callback', { nativeMode: 'batch', counterMode: 'nativeBatch', batchMs: 275 }))
  cases.push(await runCase('fixed native + 250ms batch counter, advancing mediaTime', { nativeMode: 'fixed', counterMode: 'batch', mediaMode: 'perFrame' }))
  cases.push(await runCase('fixed native + per-frame counter, mediaTime=0', { nativeMode: 'fixed' }))
  cases.push(await runCase('missing native counter + fixed quality counter, advancing mediaTime', { nativeMode: 'missing', counterMode: 'fixed', mediaMode: 'perFrame' }))
  assert.ok(cases[0].realtimeDrawFps >= 29.8)
  assert.ok(cases[1].realtimeDrawFps >= 29.8)
  assert.ok(cases[2].realtimeDrawFps >= 29.8)
  assert.ok(cases[3].realtimeDrawFps >= 3.8 && cases[3].realtimeDrawFps <= 4.2)
  assert.ok(cases[4].realtimeDrawFps >= 3.5 && cases[4].realtimeDrawFps <= 3.8)
  assert.ok(cases.find(item => item.name === 'fixed native + per-frame counter, mediaTime=0').realtimeDrawFps >= 29.8)
  const drawPaths = []
  drawPaths.push(await auditDrawPath('same-size RCAS', {}, false, 1))
  drawPaths.push(await auditDrawPath('single-pass upscale', { superResolutionOutputWidth: 64, superResolutionOutputHeight: 36 }, false, 1))
  drawPaths.push(await auditDrawPath('dual-pass EASU + RCAS', { superResolutionOutputWidth: 64, superResolutionOutputHeight: 36 }, true, 2))
  drawPaths.push(await auditConcurrentCapture())
  const report = {
    kind: 'offline-production-code-with-mocked-DOM-WebGL',
    sourceFiles: ['SuperResolutionWebGL2Renderer.ts', 'video-frame-source.ts', 'render-observation.ts', 'renderer-scheduling.test.ts'].map(name => ({ path: path.join(rendererPath, name), sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(rendererPath, name))).digest('hex') })),
    cases, drawPaths, callSites: auditCallSites(),
  }
  fs.writeFileSync(outputPath, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify({ cases: cases.map(({ steps, ...item }) => item), drawPaths, output: outputPath }, null, 2))
})().catch(error => { vi.restore(); console.error(error); process.exitCode = 1 })
