const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const { execFileSync } = require('node:child_process')
const assert = require('node:assert/strict')

// 复用真实模块加载器和既有DOM/WebGL替身；before仅替换本批三个renderer源码。
const root = process.cwd()
const changed = ['Renderers.ts', 'SuperResolutionWebGL2Renderer.ts', 'fsr1-mobile-opt-fragment.ts'].map(name => `src/player/infra/render/${name}`)
const before = new Map(changed.map(file => [path.join(root, file), execFileSync('git', ['show', `HEAD:${file}`], { encoding: 'utf8' })]))
const template = fs.readFileSync('dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs', 'utf8')
const loader = template.slice(0, template.indexOf('\n;(async () => {'))
  .replace("fs.readFileSync(file, 'utf8'), {", "(baselineSources.get(file) ?? fs.readFileSync(file, 'utf8')), {")
  + '\nmodule.exports = { harness, vi };'

async function audit(sourceOverrides) {
  const mod = { exports: {} }
  new Function('require', 'module', 'exports', 'baselineSources', loader)(require, mod, mod.exports, sourceOverrides)
  const cases = []
  for (const input of [
    { name: 'webgl equal1440p', kind: 'webgl2', options: { renderSourceWidth: 2560, renderSourceHeight: 1440 } },
    { name: 'webgl zero sharpening with larger output', kind: 'webgl2', options: { sharpness: 0, renderOutputWidth: 2560, renderOutputHeight: 1440 } },
    { name: 'SR equal1440p', kind: 'webgl2_sr', options: { renderSourceWidth: 2560, renderSourceHeight: 1440 } },
    { name: 'SR fused upscale', kind: 'webgl2_sr', options: { superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 } },
  ]) {
    const h = await mod.exports.harness(input.kind, input.options)
    try {
      const programsAtAttach = h.gl.createProgram.mock.calls.length
      let opacityWrites = 0
      let opacity = h.canvases[0].style.opacity
      Object.defineProperty(h.canvases[0].style, 'opacity', { configurable: true, get: () => opacity, set: value => { opacity = value; opacityWrites++ } })
      const offered = h.run(1000, 1000 / 60, 144)
      const uniformCount = () => ['uniform1i', 'uniform1f', 'uniform2f', 'uniform4fv'].reduce((n, name) => n + h.gl[name].mock.calls.length, 0)
      const beforeUpdate = uniformCount()
      h.renderer.update({ brightness: 100, contrast: 100, saturation: 100 })
      h.renderer.update({ targetFps: 30 })
      h.frame(1100, offered + 1)
      cases.push({
        name: input.name, offered, programsAtAttach, totalProgramAttempts: h.gl.createProgram.mock.calls.length,
        realtimeDrawCount: h.renderer.snapshotResources().realtimeDrawCount,
        gpuDrawCalls: h.gl.drawArrays.mock.calls.length,
        framebufferAllocations: h.gl.createFramebuffer.mock.calls.length,
        uniformWritesAfterRepeatedOptionsAndFpsChange: uniformCount() - beforeUpdate,
        opacityWrites,
        textureBindings: h.gl.bindTexture.mock.calls.length,
        viewportCalls: h.gl.viewport.mock.calls.length,
        activeTextureCalls: h.gl.activeTexture.mock.calls.length,
        duplicateDrawCount: h.renderer.snapshotResources().duplicateDrawCount,
      })
      assert.equal(h.renderer.snapshotResources().submitFailureCount, 0)
    }
    finally {
      h.renderer.destroy()
      mod.exports.vi.restore()
    }
  }
  return cases
}

;(async () => {
  const baseline = await audit(before)
  const current = await audit(new Map())
  for (const [index, item] of current.entries()) {
    assert.equal(item.programsAtAttach, 1)
    assert.equal(item.uniformWritesAfterRepeatedOptionsAndFpsChange, 0)
    assert.equal(item.duplicateDrawCount, 0)
    assert.equal(item.realtimeDrawCount, baseline[index].realtimeDrawCount)
    assert.ok(item.gpuDrawCalls <= baseline[index].gpuDrawCalls)
  }
  assert.equal(current[1].framebufferAllocations, 0)
  assert.equal(current[1].gpuDrawCalls, current[1].realtimeDrawCount)
  assert.equal(baseline[1].gpuDrawCalls, baseline[1].realtimeDrawCount * 2)
  assert.equal(current[2].opacityWrites, 1)
  assert.equal(current[3].opacityWrites, 1)
  const report = {
    kind: 'offline-production-code-with-mocked-DOM-WebGL-operation-counts', date: '2026-10-30',
    baselineScope: 'HEAD versions of three render modules; remaining source and harness are current',
    baseline, current,
    sourceFiles: changed.map(file => ({ path: file, beforeSha256: crypto.createHash('sha256').update(before.get(path.join(root, file))).digest('hex'), currentSha256: crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex') })),
    gpuTiming: 'unavailable', screenPresentState: 'unavailable',
  }
  const output = process.argv[2] ?? '/tmp/xbxrc-render-redundant-work-audit.json'
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify({ baseline, current, output }, null, 2))
})().catch(error => { console.error(error); process.exitCode = 1 })
