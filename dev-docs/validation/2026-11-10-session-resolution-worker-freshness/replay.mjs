import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import fs from 'node:fs/promises'
import path from 'node:path'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
const require = createRequire(import.meta.url)
const { build } = createRequire(require.resolve('vite/package.json'))('esbuild')

const directory = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(directory, '../../..')
const built = await build({ stdin: { contents: `export { DecodedVideoFrameWorkerSource } from './src/player/infra/render/decoded-video-frame-worker-source'; export { DecodedVideoFrameReader } from './src/player/infra/render/decoded-video-frame-reader'`, resolveDir: root, loader: 'ts' }, bundle: true, write: false, format: 'esm', platform: 'node', define: { 'import.meta.url': JSON.stringify(`file://${root}/src/player/infra/render/decoded-video-frame-reader.ts`) } })
const { DecodedVideoFrameWorkerSource, DecodedVideoFrameReader } = await import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].text).toString('base64')}`)
const settle = async () => { for (let i = 0; i < 8; i++) await Promise.resolve() }
const round = value => Math.round(value * 1000) / 1000

async function replay(policy, stallMs) {
  let now = 0
  const queue = []
  const frames = []
  const submitted = []
  const stallStart = 50
  const stallEnd = stallStart + stallMs
  const schedule = (at, fn) => queue.push({ at, fn })
  const mainAt = at => at >= stallStart && at < stallEnd ? stallEnd : at
  let processor
  let reader
  let reads = 0
  let ackResolve
  let source
  let stopped = false
  class Processor {
    constructor() {
      processor = this
      this.readable = { getReader: () => ({
        read: () => {
          reads++
          if (this.pending) { const frame = this.pending; this.pending = undefined; return Promise.resolve({ value: frame, done: false }) }
          return new Promise(resolve => { this.waiting = resolve })
        },
        cancel: async () => { stopped = true; this.pending?.close(); this.waiting?.({ done: true }); this.waiting = undefined },
        releaseLock() {},
      }) }
    }
    push(frame) {
      if (this.waiting) { const waiting = this.waiting; this.waiting = undefined; waiting({ value: frame, done: false }) }
      else { this.pending?.close(); this.pending = frame }
    }
  }
  const consume = frame => {
    now += 1 // 相同的同步导入/提交成本。
    submitted.push({ sequence: frame.sequence, atMs: now, sourceAgeMs: now - frame.bornAtMs })
    frame.close()
  }
  const endpoint = {
    MediaStreamTrackProcessor: Processor,
    postMessage: message => schedule(mainAt(now + 2), () => {
      if (policy === 'current') worker.onmessage?.({ data: message })
      else if (message.type === 'frame') { consume(message.frame); worker.postMessage({ type: 'ack' }) }
    }),
  }
  // 基线模型复现旧 Worker 的 read → transfer → consume → ACK → read 顺序与处理器单帧队列。
  async function baselineRead() {
    const streamReader = new Processor().readable.getReader()
    while (!stopped) {
      const { value, done } = await streamReader.read()
      if (done) return
      const acknowledged = new Promise(resolve => { ackResolve = resolve })
      endpoint.postMessage({ type: 'frame', frame: value })
      await acknowledged
    }
  }
  class Worker {
    constructor() { source = policy === 'current' ? new DecodedVideoFrameWorkerSource(endpoint, () => 100000 + now) : undefined }
    postMessage(message) {
      schedule(now + 1, () => {
        if (policy === 'current') source.handle(message)
        else if (message.type === 'start') void baselineRead()
        else if (message.type === 'ack') { ackResolve?.(); ackResolve = undefined }
        else { stopped = true; processor.pending?.close(); endpoint.postMessage({ type: 'stopped' }) }
      })
    }
    terminate() {}
  }
  const worker = new Worker()
  const saved = ['performance', 'Worker', 'MediaStreamTrackProcessor'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)])
  Object.defineProperty(globalThis, 'performance', { configurable: true, value: { timeOrigin: 100000, now: () => now } })
  Object.defineProperty(globalThis, 'Worker', { configurable: true, value: class { constructor() { return worker } } })
  Object.defineProperty(globalThis, 'MediaStreamTrackProcessor', { configurable: true, value: undefined })
  try {
    const track = Object.assign(new EventTarget(), { readyState: 'live', muted: false, clone: () => ({ stop() {} }) })
    if (policy === 'current') reader = new DecodedVideoFrameReader({ srcObject: { getVideoTracks: () => [track] } }, consume)
    else worker.postMessage({ type: 'start' })
    for (let i = 1; i <= 30; i++) schedule(i * 1000 / 60, () => {
      const frame = { sequence: i, bornAtMs: now, timestamp: 0, closed: false, close() { this.closed = true } }
      frames.push(frame)
      processor.push(frame)
    })
    const drain = async () => {
      while (queue.length) { queue.sort((a, b) => a.at - b.at); const event = queue.shift(); now = Math.max(now, event.at); event.fn(); await settle() }
    }
    await drain()
    const observation = reader?.snapshot()
    const firstFreshAfterStall = submitted.find(frame => frame.atMs >= stallEnd && frame.sourceAgeMs <= 50)
    const result = {
      policy, stallMs, generatedFrames: frames.length, submittedFrames: submitted.length, processorReads: reads,
      staleSubmissions: submitted.filter(frame => frame.sourceAgeMs > 50).length,
      sourceAgeMaxMs: round(Math.max(...submitted.map(frame => frame.sourceAgeMs))),
      freshAfterStallMs: round(firstFreshAfterStall.atMs - stallEnd),
      maxSubmitGapMs: round(Math.max(...submitted.slice(1).map((frame, i) => frame.atMs - submitted[i].atMs))),
      decodedSupply: observation,
    }
    if (reader) reader.dispose()
    else worker.postMessage({ type: 'stop' })
    await drain()
    assert.equal(frames.every(frame => frame.closed), true)
    return result
  }
  finally {
    for (const [key, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor)
      else delete globalThis[key]
    }
  }
}
const runs = []
for (const stall of [80, 175]) {
  const before = await replay('ack-then-read-model', stall)
  const after = await replay('current', stall)
  assert.equal(after.staleSubmissions, 0)
  assert.equal(after.decodedSupply.staleFrameDropCount, 1)
  assert.ok(after.freshAfterStallMs <= before.freshAfterStallMs)
  runs.push({ before, after })
}
const sourceIdentity = []
for (const file of ['src/player/infra/render/decoded-video-frame-worker-source.ts', 'src/player/infra/render/decoded-video-frame-reader.ts']) sourceIdentity.push({ file, sha256: createHash('sha256').update(await fs.readFile(path.join(root, file))).digest('hex') })
const result = { evidence: 'deterministic-synthetic-replay', sourceFps: 60, transferMs: 2, ackDeliveryMs: 1, consumeMs: 1, scope: 'current Worker/reader versus explicit legacy scheduling model; no network, GPU or physical presentation measurements', sourceIdentity, runs }
await fs.writeFile(path.join(directory, 'replay.metrics.json'), `${JSON.stringify(result, null, 2)}\n`)
process.stdout.write(`${JSON.stringify(runs.map(({ before, after }) => ({ stallMs: after.stallMs, staleSubmissionsBefore: before.staleSubmissions, staleSubmissionsAfter: after.staleSubmissions, sourceAgeMaxBeforeMs: before.sourceAgeMaxMs, sourceAgeMaxAfterMs: after.sourceAgeMaxMs, freshAfterStallBeforeMs: before.freshAfterStallMs, freshAfterStallAfterMs: after.freshAfterStallMs })), null, 2)}\n`)
