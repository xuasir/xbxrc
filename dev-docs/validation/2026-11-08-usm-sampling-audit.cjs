const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const http = require('node:http')
const { execFileSync } = require('node:child_process')

// 复用生产 renderer 与既有调度替身，导出其实际提交的 GLSL 供浏览器做像素对照。
// node dev-docs/validation/2026-11-08-usm-sampling-audit.cjs /tmp/xbxrc-usm-sampling HEAD --serve
const root = process.cwd()
const outputDir = path.resolve(process.argv[2] ?? '/tmp/xbxrc-usm-sampling')
const baselineRef = process.argv[3] ?? 'HEAD'
const baselineCommit = execFileSync('git', ['rev-parse', baselineRef], { encoding: 'utf8' }).trim()
const renderPath = 'src/player/infra/render/Renderers.ts'
const before = execFileSync('git', ['show', `${baselineRef}:${renderPath}`], { encoding: 'utf8' })
const current = fs.readFileSync(renderPath, 'utf8')
const sha = value => crypto.createHash('sha256').update(value).digest('hex')
const template = fs.readFileSync('dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs', 'utf8')
const loader = template.slice(0, template.indexOf('\n;(async () => {'))
  .replace("fs.readFileSync(file, 'utf8'), {", "(baselineSources.get(file) ?? fs.readFileSync(file, 'utf8')), {")
  + '\nmodule.exports = { harness, vi };'

function blockAfter(source, marker) {
  const start = source.indexOf(marker)
  assert.ok(start >= 0, `Missing shader block: ${marker}`)
  const open = source.indexOf('{', start)
  let depth = 1
  for (let index = open + 1; index < source.length; index++) {
    depth += Number(source[index] === '{') - Number(source[index] === '}')
    if (depth === 0) return source.slice(open + 1, index)
  }
  throw new Error(`Unclosed shader block: ${marker}`)
}

function textureCalls(fragment) {
  const count = source => (source.match(/\btexture\s*\(/g) ?? []).length
  const center = count(blockAfter(fragment, 'void main()'))
  const usm = blockAfter(fragment, 'vec3 usmBoost(')
  if (usm.includes('if (all(equal(outputResolution, sourceResolution)))'))
    return { pixelCenters: center + count(blockAfter(usm, 'if (all(equal(outputResolution, sourceResolution)))')),
      scaled: center + count(blockAfter(usm, 'else')) }
  return { pixelCenters: center + count(usm), scaled: center + count(usm) }
}

async function capture(source) {
  const mod = { exports: {} }
  new Function('require', 'module', 'exports', 'baselineSources', loader)(require, mod, mod.exports,
    new Map([[path.join(root, renderPath), source]]))
  const h = await mod.exports.harness('webgl2', { processing: 'usm', sharpness: 3, renderSourceWidth: 2560, renderSourceHeight: 1440 })
  try {
    const fragment = h.gl.shaderSource.mock.calls.map(call => call[1]).find(source => source.includes('vec3 usmBoost'))
    assert.ok(fragment)
    assert.ok(source.includes('gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)'))
    assert.ok(source.includes('gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)'))
    const offered = h.run(10000, 1000 / 60, 144)
    const resources = h.renderer.snapshotResources()
    assert.equal(resources.realtimeDrawCount, offered)
    assert.equal(resources.rateLimitDropCount, 0)
    assert.equal(resources.duplicateDrawCount, 0)
    assert.equal(resources.submitFailureCount, 0)
    assert.equal(h.gl.createProgram.mock.calls.length, 1)
    assert.equal(h.gl.createFramebuffer.mock.calls.length, 0)
    assert.equal(h.gl.texImage2D.mock.calls.filter(call => call.at(-1) === h.video).length, offered)
    return { fragment, textureCalls: textureCalls(fragment), scheduling: { durationMs: 10000, sourceFps: 60, displayHz: 144, offered, submitted: resources.realtimeDrawCount,
      rateLimitDrops: resources.rateLimitDropCount, duplicates: resources.duplicateDrawCount, failures: resources.submitFailureCount,
      programs: h.gl.createProgram.mock.calls.length, framebuffers: h.gl.createFramebuffer.mock.calls.length } }
  }
  finally {
    h.renderer.destroy()
    assert.equal(h.rafs.size + h.frames.size, 0)
    mod.exports.vi.restore()
  }
}

async function runGpuAudit(fragments, audit) {
  const status = document.getElementById('status')
  const resultNode = document.getElementById('results')
  const canvas = document.createElement('canvas')
  const gl = canvas.getContext('webgl2', { alpha: false, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: true })
  const results = []
  try {
    if (!gl) throw new Error('WebGL2 unavailable')
    gl.disable(gl.DITHER)
    const compile = (type, text) => {
      const shader = gl.createShader(type)
      gl.shaderSource(shader, text)
      gl.compileShader(shader)
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader))
      return shader
    }
    const vertex = compile(gl.VERTEX_SHADER, '#version 300 es\nlayout(location=0) in vec4 position; void main(){gl_Position=position;}')
    const programs = fragments.map(fragment => {
      const program = gl.createProgram()
      gl.attachShader(program, vertex)
      gl.attachShader(program, compile(gl.FRAGMENT_SHADER, fragment))
      gl.linkProgram(program)
      if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program))
      const uniforms = Object.fromEntries(['data', 'outputResolution', 'sourceResolution', 'filterId', 'applyColor', 'sharpenFactor', 'brightness', 'contrast', 'saturation']
        .map(name => [name, gl.getUniformLocation(program, name)]))
      return { program, uniforms }
    })
    const buffer = gl.createBuffer()
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW)
    gl.enableVertexAttribArray(0)
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0)
    const texture = gl.createTexture()
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    const image = (width, height, pattern) => {
      const pixels = new Uint8Array(width * height * 4)
      let seed = 12345
      for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
        const offset = (y * width + x) * 4
        for (let c = 0; c < 3; c++) {
          seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0
          pixels[offset + c] = pattern === 'noise' ? seed >>> 24
            : pattern === 'flat' ? [71, 129, 198][c]
              : pattern === 'checker' ? (x + y) % 2 ? 255 : 0
                : pattern === 'border' ? x === 0 || y === 0 || x === width - 1 || y === height - 1 ? 255 : 0
                  : Math.round((c === 0 ? x / Math.max(1, width - 1) : c === 1 ? y / Math.max(1, height - 1) : (x + y) / Math.max(1, width + height - 2)) * 255)
        }
        pixels[offset + 3] = 255
      }
      return pixels
    }
    const cases = []
    for (const [width, height] of [[1, 1], [2, 2], [7, 5], [32, 17], [64, 33]])
      for (const pattern of ['flat', 'noise', 'checker', 'border', 'ramp'])
        for (const strength of [0, 0.6, 1]) cases.push({ width, height, pattern, strength, filter: 1, color: false })
    for (const [width, height, outputWidth, outputHeight] of [[7, 5, 13, 11], [32, 17, 48, 29], [64, 33, 31, 15]])
      for (const pattern of ['noise', 'border', 'ramp']) cases.push({ width, height, outputWidth, outputHeight, pattern, strength: 0.6, filter: 1, color: true })
    for (const pattern of ['noise', 'ramp']) {
      cases.push({ width: 64, height: 33, pattern, strength: 0.6, filter: 2, color: true })
      cases.push({ width: 2560, height: 1440, pattern, strength: 0.6, filter: 1, color: true })
    }
    const draw = (entry, input) => {
      const { program, uniforms } = entry
      const width = input.outputWidth ?? input.width
      const height = input.outputHeight ?? input.height
      canvas.width = width
      canvas.height = height
      gl.viewport(0, 0, width, height)
      gl.useProgram(program)
      gl.uniform1i(uniforms.data, 0)
      gl.uniform2f(uniforms.outputResolution, width, height)
      gl.uniform2f(uniforms.sourceResolution, input.width, input.height)
      gl.uniform1i(uniforms.filterId, input.filter)
      gl.uniform1i(uniforms.applyColor, Number(input.color))
      gl.uniform1f(uniforms.sharpenFactor, input.strength)
      gl.uniform1f(uniforms.brightness, 100)
      gl.uniform1f(uniforms.contrast, input.color ? 105 : 100)
      gl.uniform1f(uniforms.saturation, input.color ? 105 : 100)
      gl.drawArrays(gl.TRIANGLES, 0, 3)
      const pixels = new Uint8Array(width * height * 4)
      gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels)
      const error = gl.getError()
      if (error !== gl.NO_ERROR) throw new Error('WebGL error: ' + error)
      return pixels
    }
    for (const [index, input] of cases.entries()) {
      status.textContent = '正在比对像素 ' + (index + 1) + '/' + cases.length
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, input.width, input.height, 0, gl.RGBA, gl.UNSIGNED_BYTE, image(input.width, input.height, input.pattern))
      const before = draw(programs[0], input)
      const current = draw(programs[1], input)
      let maxDifference = 0
      let changedComponents = 0
      for (let i = 0; i < before.length; i++) {
        const difference = Math.abs(before[i] - current[i])
        maxDifference = Math.max(maxDifference, difference)
        changedComponents += Number(difference > 0)
      }
      results.push({ ...input, comparedComponents: before.length, maxDifference, changedComponents, passed: maxDifference <= 1 })
      await new Promise(resolve => setTimeout(resolve, 0))
    }
    const report = { ...audit, pixelGate: results.every(result => result.passed) ? 'PASS' : 'FAIL',
      browser: navigator.userAgent, webgl: { vendor: gl.getParameter(gl.VENDOR), renderer: gl.getParameter(gl.RENDERER), version: gl.getParameter(gl.VERSION) },
      tolerance8bit: 1, maxDifference: Math.max(...results.map(result => result.maxDifference)), cases: results,
      gpuTiming: 'unmeasured', windowsHandheldFrameRate: 'unmeasured', physicalPresent: 'unavailable' }
    resultNode.textContent = JSON.stringify(report, null, 2)
    const saved = await fetch('/gpu-report', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(report) })
    if (!saved.ok) throw new Error('Result save failed: ' + saved.status)
    status.textContent = report.pixelGate + '：' + results.length + ' 组像素对照，最大差值 ' + report.maxDifference + '/255'
  }
  catch (error) {
    const report = { ...audit, pixelGate: 'FAIL', error: String(error), cases: results }
    resultNode.textContent = JSON.stringify(report, null, 2)
    status.textContent = 'FAIL：' + String(error)
    await fetch('/gpu-report', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(report) }).catch(() => {})
  }
}

;(async () => {
  const baseline = await capture(before)
  const optimized = await capture(current)
  assert.deepEqual(optimized.scheduling, baseline.scheduling)
  assert.deepEqual(baseline.textureCalls, { pixelCenters: 9, scaled: 9 })
  assert.deepEqual(optimized.textureCalls, { pixelCenters: 5, scaled: 9 })
  const audit = { kind: 'production-GLSL-pixel-comparison-and-mocked-scheduling', baselineRef, baselineCommit,
    source: { path: renderPath, baselineSha256: sha(before), currentSha256: sha(current), baselineShaderSha256: sha(baseline.fragment), currentShaderSha256: sha(optimized.fragment) },
    shaderTextureCallsPerPixel: { baseline: baseline.textureCalls.pixelCenters, optimizedAtPixelCenters: optimized.textureCalls.pixelCenters, unalignedFallback: optimized.textureCalls.scaled },
    scheduling: optimized.scheduling, measurementBoundary: 'Shader pixels run in browser WebGL2. Scheduling uses production modules with mocked DOM/WebGL. GPU performance and Windows handheld FPS require device evidence.' }
  fs.mkdirSync(outputDir, { recursive: true })
  fs.writeFileSync(path.join(outputDir, 'audit.json'), JSON.stringify(audit, null, 2) + '\n')
  const fixture = '<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>USM 像素回归</title><body><h1>USM 像素回归</h1><p id="status">正在初始化</p><pre id="results"></pre><script>'
    + '(' + runGpuAudit.toString() + ')(' + JSON.stringify([baseline.fragment, optimized.fragment]).replace(/</g, '\\u003c') + ',' + JSON.stringify(audit) + ')</script></body></html>'
  fs.writeFileSync(path.join(outputDir, 'index.html'), fixture)
  console.log(JSON.stringify({ ...audit, outputDir }, null, 2))
  if (process.argv.includes('--serve')) {
    const server = http.createServer((request, response) => {
      if (request.method === 'POST' && request.url === '/gpu-report') {
        let body = ''
        request.setEncoding('utf8')
        request.on('data', chunk => {
          body += chunk
          if (body.length > 512 * 1024) request.destroy()
        })
        request.on('end', () => {
          try {
            const report = JSON.parse(body)
            assert.deepEqual(report.source, audit.source)
            fs.writeFileSync(path.join(outputDir, 'gpu-report.json'), JSON.stringify(report, null, 2) + '\n')
            response.writeHead(200).end('saved')
          }
          catch { response.writeHead(400).end('invalid report') }
        })
      }
      else if (request.method === 'GET' && request.url === '/') {
        response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' }).end(fixture)
      }
      else response.writeHead(404).end()
    })
    server.listen(8956, '127.0.0.1', () => console.log('USM pixel fixture: http://127.0.0.1:8956/'))
  }
})().catch(error => { console.error(error); process.exitCode = 1 })
