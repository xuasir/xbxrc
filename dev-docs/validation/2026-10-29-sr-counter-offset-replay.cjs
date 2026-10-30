const fs = require('node:fs')
const path = require('node:path')
const assert = require('node:assert/strict')

// 复用此前真实模块加载器与DOM/WebGL替身，只改变受控输入的播放计数偏移。
const templatePath = path.join(process.cwd(), 'dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs')
const template = fs.readFileSync(templatePath, 'utf8')
let source = template.slice(0, template.indexOf('\n;(async () => {'))
const oldOptions = "{ nativeMode = 'perFrame', counterMode = 'perFrame', mediaMode = 'zero', batchMs = 250, refresh = 60, nativeFirst = true }"
assert.ok(source.includes(oldOptions))
source = source.replace(oldOptions, "{ nativeMode = 'perFrame', counterMode = 'perFrame', mediaMode = 'zero', batchMs = 250, refresh = 60, nativeFirst = true, counterLead = 0, leadAfterMs = 2000 }")
const oldCounter = "const counterValue = at => counterMode === 'perFrame' ? sequence : counterMode === 'fixed' ? 0 : counterMode === 'nativeBatch' ? nativeBatchCounter : Math.floor(Math.floor(at / batchMs) * batchMs * 30 / 1000)"
assert.ok(source.includes(oldCounter))
source = source.replace(oldCounter, "const counterValue = at => (counterMode === 'perFrame' ? sequence : counterMode === 'fixed' ? 0 : counterMode === 'nativeBatch' ? nativeBatchCounter : Math.floor(Math.floor(at / batchMs) * batchMs * 30 / 1000)) + (at >= leadAfterMs ? counterLead : 0)")
source = source.replace("const metadata = { mediaTime: mediaMode === 'zero' ? 0 : sequence / 30 }", "const metadata = { mediaTime: mediaMode === 'zero' ? 0 : sequence / 30, rtpTimestamp: sequence * 3000 }")
source = source.replace('name, durationMs, sourceFrames: sequence,', `name, durationMs, sourceFrames: sequence,
    steadyAcceptedMarkers: fresh.filter(item => item.at >= leadAfterMs).length,
    steadyFps: fresh.filter(item => item.at >= leadAfterMs).length * 1000 / (durationMs - leadAfterMs),
    rejectedNativeCallbacks: observations.filter(item => !item.newFrame && item.trackingSource === 'videoFrameCallback').length,
    finalIdentityState: state(),`)
source += '\nmodule.exports = { runCase };'
const moduleShim = { exports: {} }
new Function('require', 'module', 'exports', source)(require, moduleShim, moduleShim.exports)

;(async () => {
  const baseline = await moduleShim.exports.runCase('native30fps, batch counter aligned', { counterMode: 'batch' })
  const leading = await moduleShim.exports.runCase('native30fps, batch counter gains a 48-frame offset after2s', { counterMode: 'batch', counterLead: 48 })
  const pollFirst = await moduleShim.exports.runCase('native30fps, 48-frame offset, polling first', { counterMode: 'batch', counterLead: 48, nativeFirst: false })
  for (const result of [baseline, leading, pollFirst]) {
    assert.equal(result.nativeCallbacks, 300)
    assert.ok(result.steadyFps >= 29.8 && result.steadyFps <= 30.2)
    assert.ok(result.realtimeDrawCount >= 298 && result.realtimeDrawCount <= 301)
    assert.equal(result.finalIdentityState.polledFramesSinceNative, 0)
  }
  const report = {
    kind: 'offline-production-code-mocked-DOM-WebGL', counterLead: 48, leadAfterMs: 2000,
    // 修复前同输入已保存历史证据；当前运行断言修复后的交付合同。
    historicalBefore: { steadyFps: 4, realtimeDrawCount: 92, rejectedNativeCallbacks: 224, finalCounterAhead: 40 },
    sourceFiles: ['video-frame-source.ts', 'video-frame-identity-observation.ts', 'renderer-scheduling.test.ts'].map(name => ({ path: path.join(process.cwd(), 'src/player/infra/render', name), sha256: require('node:crypto').createHash('sha256').update(fs.readFileSync(path.join(process.cwd(), 'src/player/infra/render', name))).digest('hex') })),
    cases: [baseline, leading, pollFirst].map(({ steps, ...item }) => item),
  }
  const output = process.argv[2] ?? '/tmp/xbxrc-sr-counter-offset-replay.json'
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify(report, null, 2))
})().catch(error => { console.error(error); process.exitCode = 1 })
