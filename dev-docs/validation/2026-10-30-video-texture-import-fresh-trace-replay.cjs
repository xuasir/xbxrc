const fs = require('node:fs')
const path = require('node:path')
const assert = require('node:assert/strict')
const crypto = require('node:crypto')

// 复用真实帧源、renderer 与已验证替身，仅控制两路计数和回调到达顺序。
const root = process.cwd()
const template = fs.readFileSync(path.join(root, 'dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs'), 'utf8')
const shim = { exports: {} }
const prelude = template.slice(0, template.indexOf('\nasync function runCase('))
new Function('require', 'module', 'exports', prelude + '\nmodule.exports = { harness, getVideoFrameSource, vi };')(require, shim, shim.exports)
const { harness, getVideoFrameSource, vi } = shim.exports

async function run(name, nativeLead, every, nativeFirst = true) {
  const h = await harness('webgl2_sr', {}, false, 'playbackQuality')
  const source = getVideoFrameSource(h.video)
  const delivered = []
  const subscribe = () => source.request((at, metadata) => {
    delivered.push({ at, source: metadata.trackingSource, fresh: metadata.newFrame })
    subscribe()
  })
  subscribe()
  h.renderer.update({ targetFps: 0 })
  h.frameRaw(0, { presentedFrames: 0, mediaTime: 0 })
  h.supply(nativeLead ? 0 : 6120)
  h.frameRaw(102000, { presentedFrames: 6120, mediaTime: 0 })
  const debtAtResume = source.uncountedNativeFrames
  const counterBase = nativeLead ? 0 : 6120
  h.renderer.update({ targetFps: 60 })
  for (let frame = 1; frame <= 600; frame++) {
    const at = 102000 + frame * 1000 / 60
    h.supply(counterBase + frame)
    const emitNative = () => {
      if (frame % every)
        h.frameRaw(at, { presentedFrames: 6120 + frame, mediaTime: 0, rtpTimestamp: frame * 1500 })
    }
    if (nativeFirst) emitNative()
    h.raf(at + 0.2)
    if (!nativeFirst) emitNative()
  }
  const live = delivered.filter(f => f.at > 102000)
  const resources = h.renderer.snapshotResources()
  const result = {
    name, sourceFps: 60, seconds: 10, sourceFrames: 600,
    nativeCallbacks: live.filter(f => f.source === 'videoFrameCallback').length,
    acceptedNative: live.filter(f => f.source === 'videoFrameCallback' && f.fresh).length,
    acceptedPoll: live.filter(f => f.source === 'animationFrame' && f.fresh).length,
    acceptedCandidates: live.filter(f => f.fresh).length,
    debtAtResume, debtAtEnd: source.uncountedNativeFrames,
    realtimeDrawCount: resources.realtimeDrawCount,
    pacingWaitCount: resources.pacingWaitCount,
    latestOverwriteCount: resources.latestOverwriteCount,
  }
  assert.equal(resources.submitFailureCount, 0)
  assert.equal(h.gl.drawArrays.mock.calls.length, resources.realtimeDrawCount)
  h.renderer.destroy()
  for (const id of [...source.pending.keys()]) source.cancel(id)
  vi.restore()
  return result
}

;(async () => {
  const cases = []
  for (const every of [4, 6]) {
    cases.push(await run(`aligned counters; RVFC ${60 * (every - 1) / every}Hz`, 0, every))
    cases.push(await run(`native ahead 6120; RVFC ${60 * (every - 1) / every}Hz`, 6120, every))
  }
  for (const c of cases) {
    if (c.debtAtResume === 0) {
      assert.equal(c.acceptedCandidates, 600)
      assert.ok(c.realtimeDrawCount >= 599)
      assert.equal(c.latestOverwriteCount, 0)
    }
    else {
      assert.equal(c.acceptedPoll, 0)
      assert.equal(c.acceptedCandidates, c.nativeCallbacks)
      assert.ok(c.debtAtEnd >= 6119)
    }
  }
  const report = {
    kind: 'production-module-controlled-counter-offset-reproduction',
    contract: '60Hz per-frame playback counter; sparse RVFC; background native lead; mocked DOM/WebGL',
    limitation: 'Controlled counter cadence proves stale-credit behavior; device playback-counter cadence and GPU/screen presentation need independent evidence.',
    sourceFiles: ['video-frame-source.ts', 'frame-pacer.ts', 'SuperResolutionWebGL2Renderer.ts', 'renderer-scheduling.test.ts'].map(name => ({
      path: path.join(root, 'src/player/infra/render', name),
      sha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(root, 'src/player/infra/render', name))).digest('hex'),
    })),
    cases,
  }
  fs.writeFileSync(process.argv[2] ?? '/tmp/xbxrc-video-import-fresh-replay.metrics.json', JSON.stringify(report, null, 2) + '\n')
  console.log(JSON.stringify(cases))
})().catch(error => { vi.restore(); console.error(error); process.exitCode = 1 })
