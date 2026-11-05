const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')
const ts = require('typescript')

const root = path.resolve(__dirname, '../..')
const sourceFiles = [
  'src/player/infra/render/frame-pacer.ts',
  'src/player/infra/render/Renderers.ts',
  'src/player/infra/render/SuperResolutionWebGL2Renderer.ts',
  'src/player/infra/render/video-frame-source.ts',
  'src/player/infra/render/video-frame-pixel-observation.ts',
  'src/player/infra/render/render-observation.ts',
  'src/player/app/stats/StatsService.ts',
  'src/player/domain/media.ts',
  'src/streaming/runtime/browser-runtime.ts',
  'src/streaming/diagnostics.ts',
  'src/streaming/stream-panel-view-models.ts',
]
const sha256 = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const moduleObject = { exports: {} }
const compiled = ts.transpileModule(fs.readFileSync(path.join(root, sourceFiles[0]), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
vm.runInNewContext(compiled, { exports: moduleObject.exports, module: moduleObject })
const { FramePacer } = moduleObject.exports

function replay(name, sourceFps, targetFps, jitter = false) {
  const pacer = new FramePacer()
  pacer.setTargetFps(targetFps)
  const submitted = []
  const offered = sourceFps * 10
  for (let sequence = 0; sequence < offered; sequence++) {
    const atMs = 100 + (jitter ? Math.round(sequence * 1000 / sourceFps) + [-3, 0, 3][sequence % 3] : sequence * 1000 / sourceFps)
    if (pacer.admit(atMs))
      submitted.push({ sequence: sequence + 1, atMs })
  }
  const expected = Math.min(sourceFps, targetFps) * 10
  assert.ok(Math.abs(submitted.length - expected) <= 1, `${name}: budget mismatch`)
  if (sourceFps <= targetFps)
    assert.equal(submitted.length, offered, `${name}: lost a healthy candidate`)
  const maxSubmissionGapMs = Math.max(...submitted.slice(1).map((frame, index) => frame.atMs - submitted[index].atMs))
  if (sourceFps === 60 && targetFps === 30)
    assert.ok(maxSubmissionGapMs <= 34, `${name}: uneven steady cap intervals`)
  return {
    name,
    sourceFps,
    targetFps,
    offered,
    submitted: submitted.length,
    rateLimitDropped: offered - submitted.length,
    maxSubmissionGapMs,
    finalSubmittedSequence: submitted.at(-1)?.sequence,
    gate: 'PASS',
  }
}

const pacing = [
  replay('healthy30', 30, 60),
  replay('healthy60', 60, 60),
  replay('jitter60', 60, 60, true),
  replay('explicit30', 60, 30),
  ...[90, 144, 165].map(fps => replay(`fast${fps}`, fps, 60)),
]
const pacer = new FramePacer()
const burstTimes = [0, 1, 2, 2000]
const burstSubmittedSequences = burstTimes.flatMap((at, index) => pacer.admit(at) ? [index + 1] : [])
assert.deepEqual(burstSubmittedSequences, [1, 4])
pacer.setTargetFps(0)
assert.equal(pacer.admit(3000), false)
pacer.setTargetFps(60)
assert.equal(pacer.admit(3001), true)

const reportPath = process.argv[2]
const outputPath = process.argv[3]
assert.ok(reportPath && outputPath, 'Usage: node audit.cjs <Vitest JSON report> <metrics output>')
const bytes = fs.readFileSync(reportPath)
const tests = JSON.parse(bytes)
assert.equal(tests.success, true)
assert.equal(tests.numFailedTests, 0)
assert.equal(tests.numPendingTests, 0)
const suites = tests.testResults.filter(result => /(?:frame-pacer|renderer-scheduling|video-frame-source|video-frame-pixel-observation|Renderers|stream-panel-view-models)\.test\.ts$/.test(result.name))
assert.equal(suites.length, 6)
for (const suite of suites)
  assert.ok(suite.assertionResults.every(test => test.status === 'passed'), suite.name)

const result = {
  gate: 'PASS',
  mode: 'Browser latest frame',
  applicableSessions: ['home', 'cloud'],
  sourceSha256: Object.fromEntries(sourceFiles.map(file => [file, sha256(fs.readFileSync(path.join(root, file)))])),
  pacing,
  burst: { offeredAtMs: burstTimes, submittedSequences: burstSubmittedSequences, gate: 'PASS' },
  pauseResume: 'PASS',
  tests: {
    reportSha256: sha256(bytes),
    files: tests.testResults.length,
    total: tests.numTotalTests,
    passed: tests.numPassedTests,
    failed: tests.numFailedTests,
    skipped: tests.numPendingTests,
    rendererContractSuites: suites.map(suite => ({ file: path.relative(root, suite.name), cases: suite.assertionResults.length, status: suite.status })),
  },
  measurementBoundary: 'Pacer replay uses the production TS module; renderer, diagnostic, screenshot and lifecycle behavior uses production-module Vitest cases. GPU execution and physical present require a device trace.',
}
fs.writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`)
process.stdout.write(`${JSON.stringify({ gate: result.gate, tests: result.tests.passed, files: result.tests.files, pacingCases: pacing.length + 2 })}\n`)
