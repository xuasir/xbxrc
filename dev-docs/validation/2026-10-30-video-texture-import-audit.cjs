const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const { execFileSync } = require('node:child_process')

// 生产模块与受控DOM/WebGL回放；基线为本轮开始前的暂存renderer。
const root = process.cwd()
const changed = ['Renderers.ts', 'SuperResolutionWebGL2Renderer.ts'].map(name => `src/player/infra/render/${name}`)
const baseline = new Map(changed.map(file => [path.join(root, file), execFileSync('git', ['show', `:${file}`], { encoding: 'utf8' })]))
const template = fs.readFileSync('dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs', 'utf8')
const loader = template.slice(0, template.indexOf('\n;(async () => {'))
  .replace("fs.readFileSync(file, 'utf8'), {", "(baselineSources.get(file) ?? fs.readFileSync(file, 'utf8')), {")
  + '\nmodule.exports = { harness, vi };'

async function audit(overrides) {
  const module = { exports: {} }
  new Function('require', 'module', 'exports', 'baselineSources', loader)(require, module, module.exports, overrides)
  const results = []
  for (const input of [
    { name: 'ordinary equal1440p', kind: 'webgl2', options: { renderSourceWidth: 2560, renderSourceHeight: 1440 } },
    { name: 'ordinary sharpen then upscale', kind: 'webgl2', options: { renderOutputWidth: 2560, renderOutputHeight: 1440 } },
    { name: 'SR equal1440p', kind: 'webgl2_sr', options: { renderSourceWidth: 2560, renderSourceHeight: 1440 } },
    { name: 'SR fused upscale', kind: 'webgl2_sr', options: { superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 } },
    { name: 'SR EASU then RCAS', kind: 'webgl2_sr', options: { superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 }, dualPass: true },
  ]) {
    const h = await module.exports.harness(input.kind, input.options, input.dualPass)
    try {
      const offered = h.run(1000, 1000 / 60, 144)
      h.renderer.update({ targetFps: 0 })
      const textures = h.gl.createTexture.mock.calls.length
      const allocations = h.gl.texImage2D.mock.calls.filter(args => args.at(-1) === null).length
      const capture = h.renderer.captureRenderedFrame()
      h.raf(1100)
      assert.ok(await capture)
      assert.equal(h.gl.createTexture.mock.calls.length, textures)
      assert.equal(h.gl.texImage2D.mock.calls.filter(args => args.at(-1) === null).length, allocations)
      const images = h.gl.texImage2D.mock.calls.filter(args => args.at(-1) === h.video)
      const subimages = h.gl.texSubImage2D.mock.calls.filter(args => args.at(-1) === h.video)
      // 对照公开WebKit分支的完整video/level0/RGB(A)/UNSIGNED_BYTE条件。
      const eligible = images.filter(args => args.length === 6 && args[0] === h.gl.TEXTURE_2D && args[1] === 0
        && args[2] === args[3] && [h.gl.RGB, h.gl.RGBA].includes(args[3]) && args[4] === h.gl.UNSIGNED_BYTE).length
      const resources = h.renderer.snapshotResources()
      assert.equal(resources.submitFailureCount, 0)
      assert.equal(resources.duplicateDrawCount, 0)
      assert.equal(resources.captureDrawCount, 1)
      assert.equal(images.length + subimages.length, resources.realtimeDrawCount + resources.captureDrawCount)
      results.push({ name: input.name, offered, realtimeDrawCount: resources.realtimeDrawCount, captureDrawCount: resources.captureDrawCount,
        texImageVideoImports: images.length, texSubImageVideoImports: subimages.length, webkitFastPathEligibleImports: eligible,
        textureObjects: textures, nullStorageAllocations: allocations, gpuDrawCalls: h.gl.drawArrays.mock.calls.length,
        gpuObjects: resources.gpuObjects, format: input.kind === 'webgl2_sr' ? 'RGB' : 'RGBA' })
    }
    finally {
      h.renderer.destroy()
      assert.equal(h.rafs.size + h.frames.size, 0)
      module.exports.vi.restore()
    }
  }
  return results
}

;(async () => {
  const before = await audit(baseline)
  const current = await audit(new Map())
  for (const [i, item] of current.entries()) {
    assert.equal(item.realtimeDrawCount, before[i].realtimeDrawCount)
    assert.equal(item.gpuDrawCalls, before[i].gpuDrawCalls)
    assert.equal(item.textureObjects, before[i].textureObjects)
    assert.equal(item.texSubImageVideoImports, 0)
    assert.equal(item.webkitFastPathEligibleImports, item.realtimeDrawCount + item.captureDrawCount)
    assert.ok(before[i].texSubImageVideoImports > 0)
  }
  const sha = value => crypto.createHash('sha256').update(value).digest('hex')
  const report = { gate: 'PASS', kind: 'production-api-contract-replay-with-mocked-DOM-WebGL',
    baselineScope: 'index version before video import optimization', before, current,
    sourceFiles: changed.map(file => ({ path: file, beforeSha256: sha(baseline.get(path.join(root, file))), currentSha256: sha(fs.readFileSync(file)) })),
    webkitEvidence: { url: 'https://github.com/WebKit/WebKit/blob/main/Source/WebCore/html/canvas/WebGLRenderingContextBase.cpp#L3584',
      function: 'texImageSource(HTMLVideoElement&)', retrievedDate: '2026-10-30',
      fastPath: 'TexImage2D + full source + level0 + RGB/RGBA + UNSIGNED_BYTE',
      actualGpuCopySuccess: 'unavailable' },
    limitations: ['API eligibility and object reuse are verified; backend allocation, GPU-copy success, pixel output and device speed require device evidence.',
      'Performance values from the old trace remain a pre-change baseline.'] }
  const output = process.argv[2] ?? '/tmp/xbxrc-video-texture-import-audit.json'
  fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify({ gate: report.gate, before, current, output }, null, 2))
})().catch(error => { console.error(error); process.exitCode = 1 })
