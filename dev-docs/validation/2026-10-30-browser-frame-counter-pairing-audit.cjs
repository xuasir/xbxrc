const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')

const root = process.cwd()
const template = fs.readFileSync(path.join(root, 'dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs'), 'utf8')
const boundary = template.indexOf('\nasync function runCase(')
assert.ok(boundary >= 0, 'offline harness boundary missing')
const prelude = template.slice(0, boundary)
const shim = { exports: {} }
new Function('require', 'module', 'exports', prelude + '\nmodule.exports = { harness, getVideoFrameSource, vi };')(require, shim, shim.exports)
const { harness, getVideoFrameSource, vi } = shim.exports

async function run(kind, settings, diagnostics) {
  const h = await harness(kind, {}, false, 'playbackQuality')
  const source = getVideoFrameSource(h.video)
  let currentFrame = 0
  let qualityFrames = 1
  let alternateReads = 0
  const imports = []
  const delivered = []
  const oldImport = h.gl.texImage2D
  h.gl.texImage2D = vi.fn((...args) => {
    oldImport(...args)
    if (args.at(-1) === h.video) imports.push(currentFrame)
  })
  Object.defineProperty(h.video, 'webkitDecodedFrameCount', { get: () => {
    alternateReads++
    return settings.fastAlternate ? currentFrame : qualityFrames
  } })
  if (settings.cap30) h.renderer.update({ targetFps: 30 })
  h.supply(qualityFrames)
  const subscribe = () => source.request((at, frame) => {
    if (frame.newFrame) delivered.push({ at, image: currentFrame, source: frame.trackingSource })
    subscribe()
  })
  subscribe()
  if (diagnostics) source.snapshotDiagnostics()
  const skips = new Set()
  if (settings.every) {
    for (let frame = settings.every; frame <= 600; frame += settings.every)
      skips.add(settings.alignedBatch && frame % 15 === 0 ? frame - 1 : frame)
  }
  let nativeCallbacks = 0
  for (let sequence = 1; sequence <= 600; sequence++) {
    const at = sequence * 1000 / 60
    currentFrame = sequence
    qualityFrames = settings.batch ? Math.max(1, Math.floor(sequence / 15) * 15) : sequence
    h.supply(qualityFrames)
    if (settings.pollFirst) h.raf(at)
    if (!skips.has(sequence)) {
      nativeCallbacks++
      h.frameRaw(at + (settings.pollFirst ? 0.2 : 0), { presentedFrames: sequence, mediaTime: 0,
        rtpTimestamp: sequence * 1500, expectedDisplayTime: at - 5 })
    }
    if (!settings.pollFirst) h.raf(at + 0.2)
  }
  const resources = h.renderer.snapshotResources()
  const diagnostic = diagnostics ? source.snapshotDiagnostics() : undefined
  assert.equal(resources.submitFailureCount, 0)
  assert.equal(resources.duplicateDrawCount, 0)
  assert.equal(h.gl.drawArrays.mock.calls.length, resources.realtimeDrawCount)
  assert.equal(imports.length, resources.realtimeDrawCount)
  assert.equal(new Set(imports).size, imports.length, 'same source image processed more than once')
  assert.equal(new Set(delivered.map(f => f.image)).size, delivered.length, 'same source image delivered twice')
  const result = {
    renderer: kind, scenario: settings.name, diagnostics, sourceFrames: 600, sourceFps: 60, seconds: 10,
    nativeCallbacks, acceptedCandidates: delivered.length,
    acceptedPoll: delivered.filter(f => f.source === 'animationFrame').length,
    submitted: resources.realtimeDrawCount, submittedFps: resources.realtimeDrawCount/10,
    uniqueImportedImages: new Set(imports).size,
    duplicateDrawCount: resources.duplicateDrawCount, latestOverwriteCount: resources.latestOverwriteCount,
    pacingWaitCount: resources.pacingWaitCount, alternateReads,
    sampledCounterPairs: diagnostic?.sampledCount ?? 0, evidence: diagnostic?.evidence,
    callbackTiming: diagnostic?.callbackTiming, counterCadence: diagnostic?.counterCadence,
  }
  if (!diagnostics) assert.equal(alternateReads, 0)
  else {
    assert.equal(alternateReads, diagnostic.sampledCount)
    assert.ok(alternateReads <= 42, 'alternate reads exceed accepted/rejected 2Hz budget')
    assert.ok(diagnostic.evidence.length <= 8)
  }
  if (!settings.batch) assert.equal(result.submitted, settings.cap30 ? 300 : 600)
  if (settings.alignedBatch && settings.every) {
    assert.equal(result.submitted, nativeCallbacks)
    assert.equal(result.acceptedPoll, 0)
  }
  h.renderer.destroy()
  for (const id of [...source.pending.keys()]) source.cancel(id)
  assert.equal(h.frames.size + h.rafs.size, 0)
  vi.restore()
  return result
}

;(async () => {
  const scenarios = [
    { name: 'perFrame-native45', every: 4 },
    { name: 'perFrame-native50', every: 6 },
    { name: 'batch250-native45-aligned', every: 4, batch: true, alignedBatch: true },
    { name: 'batch250-native50-aligned', every: 6, batch: true, alignedBatch: true },
    { name: 'batch250-native45-pollFirst', every: 4, batch: true, pollFirst: true },
    { name: 'batch250-native50-pollFirst', every: 6, batch: true, pollFirst: true },
    { name: 'batch250-native45-fastAlternate', every: 4, batch: true, alignedBatch: true, fastAlternate: true },
    { name: 'batch250-native50-fastAlternate', every: 6, batch: true, alignedBatch: true, fastAlternate: true },
    { name: 'perFrame-native60-cap30', cap30: true },
  ]
  const results = []
  for (const kind of ['webgl2', 'webgl2_sr']) {
    for (const scenario of scenarios) {
      const off = await run(kind, scenario, false)
      const dev = await run(kind, scenario, true)
      for (const field of ['nativeCallbacks', 'acceptedCandidates', 'acceptedPoll', 'submitted', 'uniqueImportedImages', 'latestOverwriteCount'])
        assert.equal(off[field], dev[field], `${scenario.name}: diagnostic collection changed ${field}`)
      results.push(off, dev)
    }
  }
  const sourceFiles = ['video-frame-source.ts', 'video-frame-identity-observation.ts', 'Renderers.ts',
    'SuperResolutionWebGL2Renderer.ts', 'frame-pacer.ts', 'renderer-scheduling.test.ts', 'video-frame-source.test.ts']
    .map(name => {
      const file = `src/player/infra/render/${name}`
      return { path: file, sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(root, file))).digest('hex') }
    })
  const report = { kind: 'production-module-controlled-counter-pairing-replay', scenarios, results, sourceFiles,
    harnessTemplateSha256: crypto.createHash('sha256').update(template).digest('hex'),
    auditScriptSha256: crypto.createHash('sha256').update(fs.readFileSync(__filename)).digest('hex'),
    limitation: 'Controlled DOM/WebGL counters and source image identities; device counter timing, decoded-versus-visible frame identity, GPU execution and screen presentation require fresh evidence.' }
  const output = process.argv[2] ?? '/tmp/xbxrc-counter-pairing-audit.metrics.json'
  fs.writeFileSync(output, JSON.stringify(report, null, 2)+'\n')
  console.log(JSON.stringify({ output, cases: results.length, results: results.filter(r => r.diagnostics)
    .map(({ evidence, callbackTiming, counterCadence, ...r }) => r) }, null, 2))
})().catch(error => { vi.restore(); console.error(error); process.exitCode = 1 })
