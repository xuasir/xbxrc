const assert = require('node:assert/strict')
const crypto = require('node:crypto')
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')
const ts = require('typescript')

const root = path.resolve(__dirname, '../..')
const [testPath, deviceMetricsPath, outputPath, traceOverride] = process.argv.slice(2)
assert.ok(testPath && deviceMetricsPath && outputPath, 'Usage: node audit.cjs <Vitest JSON> <device metrics> <output metrics> [trace snapshot]')
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const sourceFile = 'src/player/infra/render/frame-pacer.ts'
const snapshotFile = 'dev-docs/validation/2026-11-05-frame-pacer-fixed-slot.snapshot.ts'
const testsBytes = fs.readFileSync(testPath)
const tests = JSON.parse(testsBytes)
assert.equal(tests.success, true)
assert.equal(tests.numFailedTests, 0)
assert.equal(tests.numPendingTests, 0)
const metricsBytes = fs.readFileSync(deviceMetricsPath)
const device = JSON.parse(metricsBytes)
assert.equal(device.analysisConsistencyGate, 'PASS')
const tracePath = traceOverride ?? device.trace
assert.equal(sha(fs.readFileSync(tracePath)), device.traceSha256)
const snapshotBytes = fs.readFileSync(path.join(root, snapshotFile))
assert.equal(sha(snapshotBytes), device.sourceFiles[sourceFile])

function loadPacer(bytes) {
  const moduleObject = { exports: {} }
  const compiled = ts.transpileModule(bytes.toString(), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText
  vm.runInNewContext(compiled, { exports: moduleObject.exports, module: moduleObject })
  return moduleObject.exports.FramePacer
}
const BeforePacer = loadPacer(snapshotBytes)
const CurrentPacer = loadPacer(fs.readFileSync(path.join(root, sourceFile)))

const traceRows = new Map(fs.readFileSync(tracePath, 'utf8').trim().split('\n').map(line => {
  const row = JSON.parse(line)
  return [row.seq, row]
}))
const entryPairs = device.productionPacerReplay.entryPairs.map(pair => {
  const row = traceRows.get(pair.seq)
  assert.ok(row.payload.mediaDiagnostics.frameIdentity.evidence.some(e => e.newFrame
    && e.callbackEnteredAtMs === pair.callbackEnteredAtMs && e.callbackIntervalMs === pair.callbackIntervalMs))
  const times = [pair.anchorMs, pair.previousCallbackEnteredAtMs, pair.callbackEnteredAtMs]
  const before = new BeforePacer()
  const current = new CurrentPacer()
  const beforeAdmitted = times.map(t => before.admit(t))
  const admitted = times.map(t => current.admit(t))
  assert.ok(admitted.every(Boolean))
  return { ...pair, beforeAdmitted, admitted }
})
assert.equal(entryPairs.filter(pair => pair.beforeAdmitted.includes(false)).length, device.productionPacerReplay.sameSlotEntryPairCount)

function replay(name, sourceFps, targetFps = 60, phaseShiftMs = 0, jitterMs = 0) {
  const times = Array.from({ length: sourceFps * 10 }, (_, i) => Math.round(i * 1000 / sourceFps
    + (i >= sourceFps ? phaseShiftMs + (i % 2 ? jitterMs : -jitterMs) : 0)))
  function evaluate(Pacer) {
    const pacer = new Pacer()
    pacer.setTargetFps(targetFps)
    const submitted = times.filter(t => pacer.admit(t))
    return {
      submitted: submitted.length,
      rateLimitDropped: times.length - submitted.length,
      maxSubmissionGapMs: Math.max(...submitted.slice(1).map((t, i) => t - submitted[i])),
      finalSubmittedAtMs: submitted.at(-1),
    }
  }
  const before = evaluate(BeforePacer)
  const current = evaluate(CurrentPacer)
  if (targetFps >= 60) {
    assert.equal(current.submitted, times.length, `${name}: current candidate loss`)
    assert.equal(current.finalSubmittedAtMs, times.at(-1))
  }
  else {
    assert.ok(Math.abs(current.submitted - targetFps * 10) <= 1, `${name}: cap drift`)
    if (targetFps === 30 && sourceFps === 60 && jitterMs === 0)
      assert.ok(current.maxSubmissionGapMs <= 34, `${name}: uneven 30fps cap`)
  }
  return { name, synthetic: true, sourceFps, targetFps, phaseShiftMs, jitterMs, offered: times.length, before, current, gate: 'PASS' }
}

const pacing = [
  replay('stable30', 30),
  replay('stable60', 60),
  replay('phaseShiftWith1msJitter60', 60, 60, 1000 / 60 / 2 - 1, 1),
  replay('underBudget53With4msJitter', 53, 60, 7, 4),
  ...[90, 144, 165].map(fps => replay(`fast${fps}`, fps)),
  ...[24, 30, 45, 59].map(fps => replay(`explicit${fps}`, 60, fps)),
  replay('explicit30FastJitter', 120, 30, 0, .4),
]
assert.equal(pacing.find(p => p.name === 'phaseShiftWith1msJitter60').before.submitted, 331)
assert.equal(pacing.find(p => p.name === 'underBudget53With4msJitter').before.submitted, 453)

const pacer = new CurrentPacer()
pacer.setTargetFps(30)
const burstTimes = [0, 1, 2, 2000, 2001]
assert.deepEqual(burstTimes.map(t => pacer.admit(t)), [true, false, false, true, false])
pacer.setTargetFps(0)
assert.equal(pacer.admit(3000), false)
pacer.setTargetFps(60)
assert.equal(pacer.admit(3001), true)
assert.equal(pacer.admit(3002), true)

const suites = tests.testResults.filter(result => /(?:frame-pacer|renderer-scheduling|video-frame-source|video-frame-pixel-observation|Renderers|browser-render-policy)\.test\.ts$/.test(result.name))
assert.equal(suites.length, 6)
assert.ok(suites.every(suite => suite.assertionResults.every(test => test.status === 'passed')))
const rendererSuite = suites.find(suite => suite.name.endsWith('/renderer-scheduling.test.ts'))
for (const kind of ['webgl2', 'webgl2_sr']) {
  assert.ok(rendererSuite.assertionResults.some(test => test.fullName.includes(kind)
    && test.title.includes('phase drift')))
  assert.ok(rendererSuite.assertionResults.some(test => test.fullName.includes(kind)
    && test.title.includes('adjacent native entry pairs')))
}
const sourceFiles = [sourceFile, snapshotFile, 'src/player/infra/render/Renderers.ts',
  'src/player/infra/render/SuperResolutionWebGL2Renderer.ts', 'src/player/infra/render/video-frame-source.ts',
  'src/streaming/runtime/browser-render-policy.ts']
const result = {
  gate: 'PASS',
  applicableSessions: ['home', 'cloud'],
  policy: { normalBudget: 'each valid latest candidate is submitted immediately', lowFpsBudget: 'rolling admission below 60fps; long gaps reset allowance', queuedRendererRetries: 0 },
  deviceEvidence: {
    trace: tracePath,
    originalTrace: device.trace,
    traceSha256: device.traceSha256,
    metricsSha256: sha(metricsBytes),
    pinnedPacerSourceSha256: sha(snapshotBytes),
    actualBeforeResourceAccounting: device.rendererResourceAccounting,
    entryTimestampPairCount: entryPairs.length,
    previousPairDrops: entryPairs.filter(pair => pair.beforeAdmitted.includes(false)).length,
    currentPairDrops: 0,
    limitation: 'Partial entry timestamps verify admission behavior; full device FPS/GPU/present require a new build trace.',
  },
  entryPairs,
  pacing,
  lowFpsBurst: { offeredAtMs: burstTimes, admitted: [true, false, false, true, false], gate: 'PASS' },
  pauseResume: 'PASS',
  tests: { reportSha256: sha(testsBytes), files: tests.testResults.length, total: tests.numTotalTests,
    relatedSuites: suites.map(suite => ({ file: path.relative(root, suite.name), tests: suite.assertionResults.length })) },
  sourceSha256: Object.fromEntries(sourceFiles.map(file => [file, sha(fs.readFileSync(path.join(root, file)))])),
  auditSourceSha256: sha(fs.readFileSync(__filename)),
}
fs.writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`)
process.stdout.write(`${JSON.stringify({ output: outputPath, gate: result.gate, tests: result.tests.total,
  realEntryPairs: entryPairs.length, currentPairDrops: 0,
  pacing: pacing.map(p => ({ name: p.name, beforeSubmitted: p.before.submitted, submitted: p.current.submitted, offered: p.offered })) })}\n`)
