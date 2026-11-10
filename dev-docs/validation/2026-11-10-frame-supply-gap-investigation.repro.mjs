import fs from 'node:fs/promises'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import ts from '/Users/guo.xu/Documents/code/games/xbxrc/node_modules/typescript/lib/typescript.js'
import assert from 'node:assert/strict'

const path = '/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/decoded-video-frame-reader.ts'
const source = execFileSync('git', ['show', '76a1ea69:src/player/infra/render/decoded-video-frame-reader.ts'], {
  cwd: '/Users/guo.xu/Documents/code/games/xbxrc', encoding: 'utf8',
})
const baseline = JSON.parse(await fs.readFile(new URL('./2026-11-10-frame-supply-gap-investigation.metrics.json', import.meta.url), 'utf8'))
assert.equal(createHash('sha256').update(source).digest('hex'), baseline.readerSourceSha256)
const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText
  .replaceAll('import.meta.url', JSON.stringify(`file://${path}`))
const { DecodedVideoFrameReader } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)
const original = { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout, Worker: globalThis.Worker, Processor: globalThis.MediaStreamTrackProcessor }
let now = 0
let id = 0
let timers = new Map()
globalThis.setTimeout = (callback, delay) => { const key = ++id; timers.set(key, { at: now + delay, callback }); return key }
globalThis.clearTimeout = key => timers.delete(key)
const advance = at => {
  while ([...timers.values()].some(t => t.at <= at)) {
    const [key, timer] = [...timers].filter(([, t]) => t.at <= at).sort((a, b) => a[1].at - b[1].at)[0]
    timers.delete(key); now = timer.at; timer.callback()
  }
  now = at
}
const drain = async () => { for (let i = 0; i < 8; i++) await Promise.resolve() }
const results = []
function harness() {
  let resolveRead
  let cloneStops = 0
  let readerCancels = 0
  let frameCloses = 0
  const clone = { stop() { cloneStops++ } }
  const reader = { read: () => new Promise(resolve => { resolveRead = resolve }), cancel: () => { readerCancels++; return Promise.resolve() }, releaseLock() {} }
  globalThis.MediaStreamTrackProcessor = class { readable = { getReader: () => reader } }
  const video = { srcObject: { getVideoTracks: () => [{ readyState: 'live', muted: true, clone: () => clone }] } }
  return { video, resolveFrame: () => resolveRead({ value: { timestamp: 0, close: () => frameCloses++ }, done: false }), state: () => ({ cloneStops, readerCancels, frameCloses }) }
}
try {
  // 与日志对应：reader在连接前启动，2秒后截止，7.5秒后才有首帧。
  const delayed = harness()
  let consumed = 0
  const late = new DecodedVideoFrameReader(delayed.video, frame => { consumed++; frame.close() })
  advance(2000)
  assert.equal(late.fallbackReason, 'initializationTimeout')
  assert.equal(late.mode, 'presentation')
  advance(7500)
  delayed.resolveFrame()
  await drain()
  assert.equal(consumed, 0)
  assert.equal(late.receivedFrames, 0)
  assert.deepEqual(delayed.state(), { cloneStops: 1, readerCancels: 1, frameCloses: 1 })
  results.push({ case: 'media-starts-after-two-second-deadline', mode: late.mode, fallback: late.fallbackReason, consumed, receivedFrames: late.receivedFrames, ...delayed.state() })

  now = 0; timers = new Map()
  const early = harness()
  let earlyConsumed = 0
  const timely = new DecodedVideoFrameReader(early.video, frame => { earlyConsumed++; frame.close() })
  advance(500); early.resolveFrame(); await drain(); advance(7500)
  assert.equal(timely.mode, 'main-thread')
  assert.equal(timely.receivedFrames, 1)
  assert.equal(timely.fallbackReason, undefined)
  results.push({ case: 'first-frame-before-deadline', mode: timely.mode, receivedFrames: timely.receivedFrames, consumed: earlyConsumed, ...early.state() })
  timely.dispose(); await drain()

  now = 0; timers = new Map()
  globalThis.MediaStreamTrackProcessor = undefined
  let worker
  const workerMessages = []
  globalThis.Worker = class {
    constructor() { worker = this }
    postMessage(message) { workerMessages.push(message.type) }
    terminate() {}
  }
  const workerClone = { stop() {} }
  const workerVideo = { srcObject: { getVideoTracks: () => [{ readyState: 'live', muted: true, clone: () => workerClone }] } }
  const readyOnly = new DecodedVideoFrameReader(workerVideo, () => {})
  worker.onmessage({ data: { type: 'ready' } })
  advance(2000)
  assert.equal(readyOnly.mode, 'presentation')
  assert.equal(readyOnly.fallbackReason, 'initializationTimeout')
  results.push({ case: 'worker-ready-before-media', mode: readyOnly.mode, fallback: readyOnly.fallbackReason, workerMessages })
  worker.onmessage({ data: { type: 'stopped' } })
  console.log(JSON.stringify({ gate: 'PASS', meaning: 'reproduced existing premature timeout and terminal disposal', tests: results }, null, 2))
}
finally {
  globalThis.setTimeout = original.setTimeout
  globalThis.clearTimeout = original.clearTimeout
  globalThis.Worker = original.Worker
  globalThis.MediaStreamTrackProcessor = original.Processor
}
