const fs = require('node:fs')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')

// 复用生产模块加载器及实际图像身份检查；仅DOM、GPU和时钟采用受控替身。
const helperPath = 'dev-docs/validation/2026-11-02-gpu-drawable-supply-audit.cjs'
const helper = fs.readFileSync(helperPath, 'utf8')
const boundary = helper.indexOf('\n;(async () => {')
assert.ok(boundary > 0)
const shared = { exports: {} }
new Function('require', 'module', 'exports', helper.slice(0, boundary)
  + '\nmodule.exports = { run, vi };')(require, shared, shared.exports)
const { run, vi } = shared.exports

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
    { name: 'cold171-native50', skipEvery: 6, initializationMs: 171, coldImportMs: 25 },
    { name: 'cold171-native45', skipEvery: 4, initializationMs: 171, coldImportMs: 25 },
    { name: 'cold171-native50-gpu3ms', skipEvery: 6, initializationMs: 171, coldImportMs: 25, delayMs: 3 },
    { name: 'cold171-native45-gpu3ms', skipEvery: 4, initializationMs: 171, coldImportMs: 25, delayMs: 3 },
  ]
  const results = []
  for (const kind of ['webgl2', 'webgl2_sr']) {
    for (const settings of scenarios) {
      const off = await run(kind, settings, false)
      const dev = await run(kind, settings, true)
      for (const key of ['candidates', 'submitted', 'uniqueImportedImages', 'pixelDelivered'])
        assert.equal(off[key], dev[key], `${settings.name}: dev/off ${key}`)
      if (settings.initializationMs) {
        for (const result of [off, dev]) {
          assert.equal(result.submitted, 600)
          assert.equal(result.uniqueImportedImages, 600)
          assert.equal(result.pixelDelivered, 600 / settings.skipEvery)
          assert.equal(result.preparationTiming.count, 1)
          assert.equal(result.preparationTiming.totalMs, 171)
          assert.equal(result.firstRead.initializationMs, 0)
          assert.equal(result.firstRead.durationMs, 25)
          assert.equal(result.firstRead.outcome, 'success')
        }
      }
      results.push(off, dev)
    }
    for (const settings of scenarios.slice(0, 2))
      results.push(await run(kind, settings, false, false))
  }
  const files = [helperPath, __filename, 'src/player/infra/render/video-frame-source.ts',
    'src/player/infra/render/video-frame-pixel-observation.ts', 'src/player/infra/render/Renderers.ts',
    'src/player/infra/render/SuperResolutionWebGL2Renderer.ts', 'src/player/infra/render/frame-pacer.ts',
    'src/player/infra/render/renderer-scheduling.test.ts', 'src/player/infra/render/video-frame-source.test.ts',
    'src/player/infra/render/video-frame-gpu.test-support.ts']
  const report = {
    gate: 'PASS',
    kind: 'production-module-cold-preparation-and-steady-supply-replay',
    scenarios,
    results,
    sourceFiles: files.map(path => ({ path, sha256: crypto.createHash('sha256').update(fs.readFileSync(path)).digest('hex') })),
    limitations: [
      '视频图像、GPU及计时使用受控替身；真实GPU执行、drawable与物理呈现由设备新日志验收。',
      '171ms初始化及25ms首导入场景分别记录准备和首处理，10秒供给窗口从两帧预热之后开始。',
      '重新启用冷却、重试上限及准备取消由生产帧源和renderer定向回归验证。',
    ],
  }
  const output = process.argv[2] ?? 'dev-docs/validation/2026-11-03-sr-cold-preparation.metrics.json'
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify({ gate: report.gate, cases: results.length, output,
    coldCases: results.filter(result => result.name.startsWith('cold') && !result.diagnostics)
      .map(({ kind, name, nativeCallbacks, submitted, uniqueImportedImages, pixelDelivered }) =>
        ({ kind, name, nativeCallbacks, submitted, uniqueImportedImages, pixelDelivered })) }, null, 2))
})().catch(error => {
  vi.restore()
  console.error(error)
  process.exitCode = 1
})
