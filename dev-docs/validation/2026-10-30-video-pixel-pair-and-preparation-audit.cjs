const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')

const root = process.cwd()
const template = fs.readFileSync(path.join(root, 'dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs'), 'utf8')
const boundary = template.indexOf('\nasync function runCase(')
assert.ok(boundary >= 0)
const shim = { exports: {} }
new Function('require', 'module', 'exports', template.slice(0, boundary)
  + '\nmodule.exports = { harness, getVideoFrameSource, vi };')(require, shim, shim.exports)
const { harness, getVideoFrameSource, vi } = shim.exports

async function run(kind, settings, diagnostics) {
  const h = await harness(kind, {}, false, 'playbackQuality')
  const source = getVideoFrameSource(h.video)
  let image = 0
  let readbacks = 0
  let canvasCount = 0
  const imports = []
  const candidates = []
  const canvas = { width: 0, height: 0, getContext: () => ({
    drawImage: () => h.gl.spend(settings.readCostMs ?? 0),
    getImageData: () => {
      readbacks++
      return { data: new Uint8ClampedArray(16 * 9 * 4).fill(settings.static ? 20 : image % 255) }
    },
  }) }
  const ownerDocument = { visibilityState: 'visible', createElement: () => { canvasCount++; return canvas } }
  Object.assign(h.video, { ownerDocument, readyState: 4, paused: false })
  const oldImport = h.gl.texImage2D
  h.gl.texImage2D = vi.fn((...args) => {
    oldImport(...args)
    if (args.at(-1) === h.video) imports.push(image)
  })
  if (settings.cap30) h.renderer.update({ targetFps: 30 })
  const subscribe = () => source.request((at, frame) => {
    if (frame.newFrame) candidates.push({ at, image, source: frame.trackingSource })
    subscribe()
  })
  subscribe()
  if (diagnostics) source.snapshotDiagnostics()
  const skippedFrames = new Set()
  if (!settings.perFrame) {
    for (let frame = 6; frame <= 600; frame += 6)
      skippedFrames.add(!settings.pollFirst && frame % 15 === 0 ? frame - 1 : frame)
  }
  for (let frame = 1; frame <= 600; frame++) {
    image = frame
    const at = frame * 1000 / 60
    h.supply(settings.perFrame ? frame : Math.max(1, Math.floor(frame / 15) * 15))
    ownerDocument.visibilityState = settings.hidden && frame >= 201 && frame < 261 ? 'hidden' : 'visible'
    if (settings.pollFirst) h.raf(at)
    const skipped = skippedFrames.has(frame)
    if (!skipped) h.frameRaw(at + (settings.pollFirst ? 0.2 : 0), { presentedFrames: frame, mediaTime: 0 })
    if (!settings.pollFirst) h.raf(at + 0.2)
  }
  const resources = h.renderer.snapshotResources()
  const pixels = diagnostics ? source.snapshotDiagnostics().videoPixels : undefined
  assert.equal(new Set(imports).size, imports.length)
  assert.equal(new Set(candidates.map(item => item.image)).size, candidates.length)
  assert.equal(resources.realtimeDrawCount, imports.length)
  assert.equal(resources.duplicateDrawCount, 0)
  assert.equal(resources.submitFailureCount, 0)
  if (diagnostics) {
    assert.equal(pixels.readCount, readbacks)
    assert.ok(readbacks <= 40, 'pixel readback budget exceeded')
    assert.ok(pixels.pairs.length <= 8)
    assert.equal(pixels.disabledReason, undefined)
    for (const pair of pixels.pairs) {
      assert.ok(pair.gapMs >= 8 && pair.gapMs <= 50)
      assert.ok(pair.readFinishedAtMs >= pair.readStartedAtMs)
      assert.equal(pair.changedPixelCount, settings.static ? 0 : 144)
    }
    if (!settings.perFrame && !settings.pollFirst) {
      assert.ok(pixels.pairCount > 0, 'missing RVFC pixel pairs absent')
      assert.equal(pixels.changedPairCount, settings.static ? 0 : pixels.pairCount)
    }
  }
  else assert.equal(readbacks + canvasCount, 0)
  const result = { renderer: kind, scenario: settings.name, diagnostics, sourceFrames: 600, seconds: 10,
    candidates: candidates.length, acceptedPoll: candidates.filter(item => item.source === 'animationFrame').length,
    submitted: imports.length, submittedFps: imports.length / 10, uniqueImages: new Set(imports).size,
    readbacks, canvasCount, pixels, preparation: resources.preparation }
  h.renderer.destroy()
  for (const id of [...source.pending.keys()]) source.cancel(id)
  assert.equal(source.snapshotResources().pixelProbeCanvases + source.snapshotResources().pixelProbePendingPairs, 0)
  assert.equal(h.frames.size + h.rafs.size, 0)
  vi.restore()
  return result
}

;(async () => {
  const scenarios = [
    { name: 'batch250-native50-moving' },
    { name: 'batch250-native50-static', static: true },
    { name: 'batch250-native50-moving-read0.25ms', readCostMs: 0.25 },
    { name: 'batch250-native50-pollFirst', pollFirst: true },
    { name: 'batch250-native50-hidden', hidden: true },
    { name: 'perFrame-native60-cap30', perFrame: true, cap30: true },
  ]
  const results = []
  for (const kind of ['webgl2', 'webgl2_sr']) {
    for (const settings of scenarios) {
      const off = await run(kind, settings, false)
      const dev = await run(kind, settings, true)
      for (const field of ['candidates', 'acceptedPoll', 'submitted', 'uniqueImages'])
        assert.equal(off[field], dev[field], `${kind}/${settings.name}: diagnostics changed ${field}`)
      results.push(off, dev)
    }
  }
  const sourceFiles = ['video-frame-source.ts', 'video-frame-pixel-observation.ts', 'render-observation.ts',
    'Renderers.ts', 'SuperResolutionWebGL2Renderer.ts', 'renderer-scheduling.test.ts', 'video-frame-source.test.ts']
    .map(name => `src/player/infra/render/${name}`).concat(['src/player/app/media/PlaybackService.ts', 'src/player/app/media/PlaybackService.test.ts'])
    .map(file => ({ path: file, sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(root, file))).digest('hex') }))
  const output = process.argv[2] ?? '/tmp/xbxrc-pixel-pair-and-preparation.metrics.json'
  const inputs = ['src', 'src-tauri/src', 'src-tauri/build.rs', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml',
    'crates', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'package.json', 'pnpm-lock.yaml', 'vite.config.ts',
    'tsconfig.json', 'tsconfig.node.json', 'tsconfig.app.json']
  const paths = [...new Set(execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', ...inputs],
    { cwd: root, encoding: 'utf8' }).split('\0').filter(Boolean))].sort()
  const fingerprint = crypto.createHash('sha256')
  const length = value => { const buffer = Buffer.alloc(8); buffer.writeBigUInt64LE(BigInt(value)); return buffer }
  for (const file of paths) {
    const name = Buffer.from(file)
    fingerprint.update(length(name.length)).update(name)
    if (fs.existsSync(path.join(root, file))) {
      const content = fs.readFileSync(path.join(root, file))
      fingerprint.update(Buffer.from([1])).update(length(content.length)).update(content)
    }
    else fingerprint.update(Buffer.from([0]))
  }
  fs.writeFileSync(output, JSON.stringify({ kind: 'production-module-pixel-pair-and-preparation-replay', results, scenarios, sourceFiles,
    buildSourceFingerprint: { sha256: fingerprint.digest('hex'), inputCount: paths.length },
    checks: { controlledReplay: 'PASS', offDevDelivery: 'PASS', pixelBudget: 'PASS', uniqueImports: 'PASS', shutdown: 'PASS' },
    auditScriptSha256: crypto.createHash('sha256').update(fs.readFileSync(__filename)).digest('hex'),
    harnessTemplateSha256: crypto.createHash('sha256').update(template).digest('hex'),
    limitation: 'Controlled video pixels and browser scheduling; live canvas2d drawable pixels, WebGL import, GPU cost and physical presentation require device evidence.' }, null, 2) + '\n')
  console.log(JSON.stringify({ output, cases: results.length, results: results.filter(item => item.diagnostics).map(item => ({
    renderer: item.renderer, scenario: item.scenario, submittedFps: item.submittedFps, readbacks: item.readbacks,
    pairCount: item.pixels.pairCount, changedPairCount: item.pixels.changedPairCount, readDurationMaxMs: item.pixels.readDurationMaxMs,
  })) }, null, 2))
})().catch(error => { vi.restore(); console.error(error); process.exitCode = 1 })
