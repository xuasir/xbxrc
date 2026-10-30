const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')

// 从仓库根目录运行；读取生产策略模块，旧策略取本轮实施前的暂存版本。
const root = process.cwd()
const trace = process.argv[2] ?? '/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793329037119-1.jsonl'
const output = process.argv[3] ?? '/tmp/xbxrc-display-deficit-policy-replay.json'
const relative = 'src/streaming/runtime/browser-render-health.ts'
const ts = require(path.join(root, 'node_modules/typescript'))
const baselineSource = execFileSync('git', ['show', `:${relative}`], { cwd: root, encoding: 'utf8' })
function createLoader(overrides = new Map()) {
  const cache = new Map()
  function load(file) {
    if (!path.extname(file)) file += '.ts'
    if (cache.has(file)) return cache.get(file).exports
    const module = { exports: {} }
    cache.set(file, module)
    const source = ts.transpileModule(overrides.get(file) ?? fs.readFileSync(file, 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    }).outputText
    new Function('require', 'exports', 'module', source)(
      spec => spec.startsWith('.') ? load(path.resolve(path.dirname(file), spec)) : require(spec),
      module.exports, module,
    )
    return module.exports
  }
  return load
}
const file = path.join(root, relative)
const baseline = createLoader(new Map([[file, baselineSource]]))(file)
const current = createLoader()(file)
const rows = fs.readFileSync(trace, 'utf8').trim().split('\n').map(line => JSON.parse(line))
const consumed = rows.filter(row => row.event === 'browserObservationConsumed'
  && row.payload.consumer === 'supervision' && row.payload.rendererGeneration === 3
  && row.payload.facts.rendererKind === 'webgl2_sr'
  && row.payload.facts.sourceWidth === 2560 && row.payload.facts.sourceHeight === 1440)
const coverageKeys = ['receivedCoverageMs', 'decodeCoverageMs', 'videoCoverageMs', 'callbackCoverageMs', 'rendererCoverageMs']
function traceStats(row) {
  const p = row.payload, f = p.facts
  return {
    statsSampleId: p.sampleId, statsSampledAtMs: p.sampledAtMs, statsSampleAgeMs: p.observedAtMs - p.sampledAtMs,
    statsObservationState: p.state, statsPeerGeneration: p.peerGeneration,
    rendererGeneration: p.rendererGeneration, rendererKind: f.rendererKind, rendererTargetFps: 60,
    inboundVideoFps: f.receivedFps, decodeFps: f.decodeFps, callbackFps: f.callbackFps,
    videoPresentFps: f.videoPresentFps, rendererSubmitFps: f.rendererSubmitFps,
    browserStatsWindows: f.windows,
  }
}
function replayTrace(policy) {
  const health = new policy.BrowserRenderHealth()
  const causes = {}, actions = []
  for (const row of consumed) {
    const stats = traceStats(row)
    if (coverageKeys.every(key => stats.browserStatsWindows.short[key] >= 2999)) {
      const cause = policy.resolveBrowserRenderCause(stats)
      causes[cause] = (causes[cause] ?? 0) + 1
    }
    // 旧日志保持旧renderer；只结算首次反事实动作，动作后的改善由受控输入单列。
    if (!actions.length) {
      const action = health.observe(stats, row.payload.observedAtMs)
      if (action) actions.push({ seq: row.seq, sampledAtMs: stats.statsSampledAtMs, ...action })
    }
  }
  return { causes, firstAction: actions[0] ?? null }
}
const traceReplay = { baseline: replayTrace(baseline), current: replayTrace(current) }
assert.ok(traceReplay.baseline.causes.renderStable > 0)
assert.equal(traceReplay.baseline.firstAction, null)
assert.equal(traceReplay.current.causes.renderStable ?? 0, 0)
assert.equal(traceReplay.current.firstAction.reason, 'sustainedFrameDeliveryDeficit')
assert.equal(traceReplay.current.firstAction.evidence.coverageMs, 3000)
assert.ok(traceReplay.current.firstAction.evidence.sustainedMs >= 3000)

function facts(at, { generation = 1, generationAt = 0, kind = 'webgl2_sr', sourceFps = 60, targetFps = 60, callbacks = sourceFps, outputFps = callbacks } = {}) {
  const window = length => ({
    durationMs: length, ...Object.fromEntries(coverageKeys.map(key => [key, key === 'rendererCoverageMs' ? Math.min(length, at - generationAt) : length])),
    inboundFpsAvg: sourceFps, decodeFpsAvg: sourceFps, videoPresentFpsAvg: sourceFps,
    callbackFpsAvg: callbacks, rendererSubmitFpsAvg: outputFps, renderDrawDurationMsAvg: 15,
  })
  return { statsObservationState: 'fresh', statsSampleId: `1:${at}`, statsSampledAtMs: at, statsSampleAgeMs: 0,
    rendererGeneration: generation, rendererKind: kind, rendererTargetFps: targetFps,
    browserStatsWindows: { short: window(3000), long: window(15000) } }
}
const health = new current.BrowserRenderHealth()
const actions = [], effects = []
for (let at = 3000; at <= 60000; at += 1000) {
  const phase = at <= 6000 ? { callbacks: 44 }
    : at <= 24000 ? { generation: 2, generationAt: 6000, kind: 'video', callbacks: 52, outputFps: 60 }
    : at <= 27000 ? { generation: 3, generationAt: 24000, callbacks: 55 }
    : { generation: 4, generationAt: 27000, kind: 'video', callbacks: 52, outputFps: 60 }
  const before = health.effect
  const action = health.observe(facts(at, phase), at)
  if (action) actions.push({ atMs: at, ...action })
  if (before !== health.effect) effects.push({ atMs: at, effect: health.effect })
}
assert.deepEqual(actions.map(a => [a.atMs, a.fallback, a.reason]), [
  [6000, true, 'sustainedFrameDeliveryDeficit'],
  [24000, false, 'healthyEnhancementTrial'],
  [27000, true, 'enhancementTrialFailed'],
])
assert.ok(effects.some(e => e.atMs === 9000 && e.effect === 'improved'))
assert.equal(health.fallback, true)
const protectedCases = [
  { name: '30fps source', input: { sourceFps: 30 } },
  { name: 'user 30fps budget', input: { targetFps: 30, callbacks: 30 } },
]
for (const item of protectedCases) {
  const guard = new current.BrowserRenderHealth()
  for (let at = 3000; at <= 30000; at += 1000) {
    const stats = facts(at, item.input)
    assert.equal(current.resolveBrowserRenderCause(stats), 'renderStable')
    assert.equal(guard.observe(stats, at), undefined)
  }
}
const sha = value => crypto.createHash('sha256').update(value).digest('hex')
const report = {
  trace, traceSha256: sha(fs.readFileSync(trace)), consumedSamples: consumed.length,
  source: { path: relative, baselineScope: 'index version before this fix', baselineSha256: sha(baselineSource), currentSha256: sha(fs.readFileSync(file)) },
  traceReplay, syntheticFallbackTrial: { actions, effects, servingFallback: health.fallback },
  protectedCases: protectedCases.map(item => ({ name: item.name, gate: 'PASS' })), gate: 'PASS',
  limitations: ['Trace replay verifies classification and first action only.', 'Post-action supply is synthetic; device throughput, GPU execution and physical scanout require new device evidence.'],
}
fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n')
console.log(JSON.stringify(report, null, 2))
