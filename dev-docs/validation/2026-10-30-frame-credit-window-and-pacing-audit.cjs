const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')

const root = process.cwd()
const baseFiles = ['src/player/infra/render/video-frame-source.ts', 'src/player/infra/render/frame-pacer.ts']
const savedBaseline = process.argv[3] ? JSON.parse(fs.readFileSync(process.argv[3], 'utf8')).baselineModules : undefined
const beforeSources = Object.fromEntries(baseFiles.map(file => [file, savedBaseline?.[file] ?? execFileSync('git', ['show', `:${file}`], { cwd: root, encoding: 'utf8' })]))
const template = fs.readFileSync(path.join(root, 'dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs'), 'utf8')
let prelude = template.slice(0, template.indexOf('\nasync function runCase('))
assert.ok(prelude.includes("ts.transpileModule(fs.readFileSync(file, 'utf8')"))
prelude = prelude.replace("ts.transpileModule(fs.readFileSync(file, 'utf8')", 'ts.transpileModule(moduleSource(file)')

function environment(before) {
  const shim = { exports: {} }
  const moduleSource = file => {
    const relative = path.relative(root, file)
    return before && beforeSources[relative] !== undefined ? beforeSources[relative] : fs.readFileSync(file, 'utf8')
  }
  new Function('require', 'module', 'exports', 'moduleSource', prelude + '\nmodule.exports = { harness, getVideoFrameSource, vi };')(require, shim, shim.exports, moduleSource)
  return shim.exports
}

async function run(env, kind, mode, every = 4) {
  const h = await env.harness(kind, {}, false, 'playbackQuality')
  const source = env.getVideoFrameSource(h.video)
  const deliveries = []
  const subscribe = () => source.request((at, frame) => {
    deliveries.push({ at, source: frame.trackingSource, fresh: frame.newFrame })
    subscribe()
  })
  subscribe()
  let start = 0
  if (mode === 'background') {
    h.renderer.update({ targetFps: 0 })
    for (let frame = 0; frame <= 6120; frame++) {
      h.supply(0)
      h.frameRaw(frame * 1000 / 60, { presentedFrames: frame, mediaTime: 0 })
    }
    start = 102000
    h.renderer.update({ targetFps: 60 })
  }
  else if (mode === 'cap30') {
    h.renderer.update({ targetFps: 30 })
  }
  const debtAtStart = source.uncountedNativeFrames
  const sourceFps = mode === 'cap30' ? 120 : 60
  const total = sourceFps * 10
  let steadyBase = 0
  for (let frame = 0; frame < total; frame++) {
    const at = mode === 'quantized' || mode === 'cap30'
      ? Math.round(frame * 1000 / sourceFps)
      : start + (frame + 1) * 1000 / sourceFps
    const sequence = frame + 1
    h.supply(sequence)
    const sparse = mode === 'background' || mode === 'originJump'
    if (!sparse || sequence % every) {
      const native = mode === 'background' || (mode === 'originJump' && sequence > 1) ? 6120 + sequence : sequence
      h.frameRaw(at, { presentedFrames: native, mediaTime: 0, rtpTimestamp: sequence * 1500 })
    }
    h.raf(at + (mode === 'quantized' || mode === 'cap30' ? 0 : 0.2))
    if (sequence === sourceFps * 2)
      steadyBase = h.renderer.snapshotResources().realtimeDrawCount
  }
  const resources = h.renderer.snapshotResources()
  const live = deliveries.filter(f => f.at > start && f.fresh)
  assert.equal(resources.submitFailureCount, 0)
  assert.equal(resources.duplicateDrawCount, 0)
  assert.equal(h.gl.drawArrays.mock.calls.length, resources.realtimeDrawCount)
  const result = {
    kind, mode, sourceFps, sourceFrames: total, seconds: 10,
    nativeFps: mode === 'background' || mode === 'originJump' ? sourceFps * (every - 1) / every : sourceFps,
    acceptedCandidates: live.length, acceptedPoll: live.filter(f => f.source === 'animationFrame').length,
    realtimeDrawCount: resources.realtimeDrawCount, realtimeDrawFps: resources.realtimeDrawCount / 10,
    steadyDrawFps: (resources.realtimeDrawCount - steadyBase) / 8,
    pacingWaitCount: resources.pacingWaitCount, latestOverwriteCount: resources.latestOverwriteCount,
    debtAtStart, debtAtEnd: source.uncountedNativeFrames,
    expiredNativeFramesTotal: source.expiredNativeFramesTotal ?? 0,
    nativeCreditWindowExpirationCount: source.nativeCreditWindowExpirationCount ?? 0,
    imports: h.gl.texImage2D.mock.calls.filter(args => args.at(-1) === h.video).length,
    failures: resources.submitFailureCount,
  }
  h.renderer.destroy()
  for (const id of [...source.pending.keys()]) source.cancel(id)
  assert.equal(h.frames.size + h.rafs.size, 0)
  env.vi.restore()
  return result
}

;(async () => {
  const report = { kind: 'production-module-controlled-replay', before: [], after: [] }
  for (const before of [true, false]) {
    const env = environment(before)
    const output = before ? report.before : report.after
    for (const kind of ['webgl2', 'webgl2_sr']) {
      output.push(await run(env, kind, 'background', 4))
      output.push(await run(env, kind, 'background', 6))
      output.push(await run(env, kind, 'originJump'))
      output.push(await run(env, kind, 'quantized'))
      output.push(await run(env, kind, 'cap30'))
    }
  }
  for (const c of report.after) {
    if (c.mode === 'cap30') {
      assert.ok(c.realtimeDrawCount >= 299 && c.realtimeDrawCount <= 301)
    }
    else if (c.mode === 'originJump') {
      assert.ok(c.steadyDrawFps >= 59.8 && c.steadyDrawFps <= 60.2)
      assert.ok(c.expiredNativeFramesTotal >= 6000)
    }
    else {
      assert.equal(c.realtimeDrawCount, 600)
      assert.equal(c.latestOverwriteCount, 0)
    }
  }
  for (const c of report.before) {
    if (c.mode === 'quantized') assert.equal(c.realtimeDrawCount, 400)
    if (c.mode === 'background') assert.equal(c.realtimeDrawCount, c.nativeFps * 10)
  }
  report.sourceFiles = [...baseFiles, 'src/player/app/media/PlaybackService.ts', 'src/player/infra/render/video-frame-identity-observation.ts', 'src/player/infra/render/video-frame-source.test.ts', 'src/player/infra/render/frame-pacer.test.ts', 'src/player/infra/render/renderer-scheduling.test.ts'].map(file => ({
    path: file, sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(root, file))).digest('hex'),
    baselineSha256: beforeSources[file] === undefined ? undefined : crypto.createHash('sha256').update(beforeSources[file]).digest('hex'),
  }))
  // 保存精确基线，后续提交后仍可按相同受控输入复算历史结果。
  report.baselineModules = beforeSources
  report.auditScriptSha256 = crypto.createHash('sha256').update(fs.readFileSync(__filename)).digest('hex')
  report.limitation = 'DOM/WebGL and clocks are controlled; device GPU, pixel identity, network recovery and screen presentation require fresh runtime evidence. Baseline modules come from the current Git index.'
  fs.writeFileSync(process.argv[2] ?? '/tmp/xbxrc-frame-credit-window-and-pacing.metrics.json', JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify({ before: report.before, after: report.after }, null, 2))
})().catch(error => { console.error(error); process.exitCode = 1 })
