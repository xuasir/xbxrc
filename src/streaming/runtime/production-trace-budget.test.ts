import type { RuntimeTraceRecordEventParams } from '@shared/rpc/runtimeTrace'
import { readFileSync, writeFileSync } from 'node:fs'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { inputUiTrace } from '../../navigation/core/input-trace'
import { productionTraceEvents } from '../../shared/diagnostics/production-trace-contract'
import { LatencyWindow } from '../../shared/latency-window'
import { facts, observation } from './__fixtures__/playback'
import { createBrowserPlaybackTrace } from './browser-playback-trace'
import { createBrowserRuntimeTrace } from './browser-runtime-trace'

const encoder = new TextEncoder()
function writerRow(params: RuntimeTraceRecordEventParams, sequence: number) {
  return { schemaVersion: 3, seq: sequence, traceRunId: '1794116093651-10000', tsMs: 1794116093651 + Number((params.payload as any).observedAtMs), traceMode: 'production', traceProfile: 'production', domain: 'streaming-runtime-host', ...params }
}
afterEach(() => vi.restoreAllMocks())

describe('production trace full-path budgets', () => {
  it('publishes complete 30min health and sustained anomaly payloads within JSONL and cost budgets', async () => {
    let at = 0
    vi.spyOn(performance, 'now').mockImplementation(() => at)
    const rows: Array<ReturnType<typeof writerRow>> = []
    const producer = createBrowserRuntimeTrace({
      context: () => ({
        sessionId: '12345678-1234-1234-1234-123456789012',
        runtimeGeneration: 123,
        clientGeneration: 123,
      }),
      write: async (params) => {
        rows.push(writerRow(params, rows.length + 1))
        return {
          accepted: true,
          disposition: 'enqueued',
        }
      },
    })
    producer.configure('production')
    const durations: number[] = []
    const timer = new LatencyWindow()
    for (let i = 0; i < 256; i++) timer.record(0.5 + i / 256)
    const distribution = timer.snapshot()
    const timing = { duration: distribution, upload: distribution, submit: distribution, interval: distribution, receiveToSubmit: distribution, readToSubmit: distribution }
    const decodedSupply = {
      schemaVersion: 1 as const,
      clockDomain: 'performanceEpoch' as const,
      observedAtMs: 1800000,
      receivedMessages: 108000,
      deliveredFrames: 107000,
      staleFrameDropCount: 1000,
      invalidClockCount: 0,
      deliveryBudgetMs: 50,
      workerReadFrames: 110000,
      workerSentFrames: 108000,
      workerOverwriteCount: 2000,
      workerAckCount: 107999,
      lastReadSequence: 110000,
      receiveAgeMs: 100,
      workerObservationAgeMs: 1800000,
      timing: { readWait: distribution, pendingWait: distribution, transfer: distribution, consume: distribution, receiveInterval: distribution, ackRoundTrip: distribution, ackDelivery: distribution },
    }
    const backend = { writtenRows: 500000, writtenBytes: 16777216, flushedSequence: 500000, sequence: 500001, ioFailures: 0, pendingRows: 1, pendingBytes: 4096, peakPendingRows: 8192, peakPendingBytes: 8388608, essentialDropped: 0, keyDropped: 0, debugDropped: 0, rawDropped: 0, oversizedDropped: 0, configurationDropped: 0, ioDropped: 0 }
    const trace = createBrowserPlaybackTrace({ enabled: () => true, now: () => at, facts: () => facts, trace: () => ({ ...producer.snapshot(), backend, backendSampleAgeMs: 29000 }), write: (event, payload, options) => producer.record(event, payload, options) })
    trace.begin()
    for (at = 0; at <= 1800000; at += 250) {
      const current = observation(at, { rendererTiming: timing, rendererDecodedFrameSupply: decodedSupply, rendererFrameSource: 'worker', rendererGpuTiming: { source: 'EXT_disjoint_timer_query_webgl2', state: 'ready', queryCount: 108000, disjointCount: 0, duration: distribution }, browserStatsWindows: { short: { sampleCount: 12, durationMs: 3000, inboundFpsAvg: 60, decodeFpsAvg: 60, videoPresentFpsAvg: 60, callbackFpsAvg: 60, rendererSubmitFpsAvg: 60, receivedCoverageMs: 3000, decodeCoverageMs: 3000, videoCoverageMs: 3000, callbackCoverageMs: 3000, rendererCoverageMs: 3000 }, long: { sampleCount: 60, durationMs: 15000, inboundFpsAvg: 60, decodeFpsAvg: 60, videoPresentFpsAvg: 60, callbackFpsAvg: 60, rendererSubmitFpsAvg: 60, receivedCoverageMs: 15000, decodeCoverageMs: 15000, videoCoverageMs: 15000, callbackCoverageMs: 15000, rendererCoverageMs: 15000 } }, maxCallbackIntervalMs: at >= 900000 ? 150 : 16.67, renderCause: 'healthy', videoPresentAgeMs: 5, rendererSubmitAgeMs: 5, renderDrawDurationMsAvg: 1, renderUploadDurationMsAvg: 0.5 })
      const start = process.hrtime.bigint()
      trace.observe(current)
      trace.tick()
      if (at > 0 && at % 1000 === 0)
        durations.push(Number(process.hrtime.bigint() - start) / 1e6)
      expect(trace.snapshot().ringSamples).toBeLessThanOrEqual(60)
      await Promise.resolve()
    }
    trace.end('sessionEnded')
    await producer.flush()
    const health = rows.filter(row => row.event === 'browserPlaybackHealthObserved')
    expect(health.length, JSON.stringify(producer.snapshot())).toBeGreaterThanOrEqual(1800)
    for (const row of rows) {
      const descriptor = productionTraceEvents.find(item => item.event === row.event)!
      expect(encoder.encode(`${JSON.stringify(row)}\n`).length, row.event).toBeLessThanOrEqual(descriptor.maxRowBytes)
    }
    expect(producer.snapshot()).toMatchObject({ dropped: 0, producerRowOversized: 0, ipcFailed: 0, inFlight: 0, pending: 0 })
    const normal = rows.filter(row => !row.event.startsWith('browserPlaybackAnomaly'))
    const normalBytesPerMinute = normal.reduce((sum, row) => sum + encoder.encode(`${JSON.stringify(row)}\n`).length, 0) / 30
    expect(normalBytesPerMinute).toBeLessThanOrEqual(327680)
    const anomalyBytes = rows.filter(row => row.event === 'browserPlaybackAnomalySample').reduce((sum, row) => sum + encoder.encode(`${JSON.stringify(row)}\n`).length, 0)
    expect(anomalyBytes / 15).toBeLessThanOrEqual(131072)
    durations.sort((a, b) => a - b)
    const p95Ms = durations[Math.ceil(durations.length * 0.95) - 1]!
    // 成本门由独立 runner 执行；全量功能测试并发时继续验证全部硬字节与覆盖预算。
    if (process.env.PRODUCTION_TRACE_VERIFY_COST === '1')
      expect(p95Ms).toBeLessThanOrEqual(0.5)
    if (process.env.PRODUCTION_TRACE_METRICS_PATH)
      writeFileSync(process.env.PRODUCTION_TRACE_METRICS_PATH, `${JSON.stringify({ healthRecords: health.length, normalBytesPerMinute, anomalyBytesPerMinute: anomalyBytes / 15, healthEncodingP95Ms: p95Ms, maxHealthRowBytes: Math.max(...health.map(row => encoder.encode(`${JSON.stringify(row)}\n`).length)), producer: producer.snapshot() }, null, 2)}\n`)
    if (process.env.PRODUCTION_TRACE_FIXTURE_PATH)
      writeFileSync(process.env.PRODUCTION_TRACE_FIXTURE_PATH, `${rows.map(row => JSON.stringify(row)).join('\n')}\n`)
  })

  it('keeps menu health at 1Hz through the actual producer under continuous navigation and confirmations', async () => {
    let at = 0
    vi.spyOn(performance, 'now').mockImplementation(() => at)
    const rows: Array<ReturnType<typeof writerRow>> = []
    const producer = createBrowserRuntimeTrace({
      context: () => ({
        sessionId: '12345678-1234-1234-1234-123456789012',
        runtimeGeneration: 123,
        clientGeneration: 123,
      }),
      write: async (params) => {
        rows.push(writerRow(params, rows.length + 1))
        return {
          accepted: true,
          disposition: 'enqueued',
        }
      },
    })
    producer.configure('production')
    const dispose = inputUiTrace.attach({ enabled: () => true, write: (event, payload, essential) => producer.record(event, payload, { intervalManaged: true, importance: essential ? 'essential' : undefined }), input: () => ({ owner: 'ui', backend: 'browser', activeDeviceId: 'private-device', exclusiveCapture: false, sampling: { sampledAtMs: at, pollingHz: 300, publishedHz: 40, gap: { retained: 256, p95Ms: 4, p99Ms: 10, maxMs: 25 } } }), anomaly: () => {} })
    try {
      inputUiTrace.scope('stream.diagnostics-menu-sheet', true)
      inputUiTrace.navigationState('stream.diagnostics-menu-sheet')
      for (let second = 1; second <= 1800; second++) {
        at = second * 1000
        for (let i = 0; i < 100; i++) inputUiTrace.withIntent('down', 'gamepad', () => inputUiTrace.navigationResult('down', true))
        for (let i = 0; i < 10; i++) inputUiTrace.withIntent('action', 'gamepad', () => inputUiTrace.activation('gamepad', 'fullscreen', undefined, () => inputUiTrace.result('operationCompleted')))
        inputUiTrace.tick()
        await producer.flush()
      }
      const health = rows.filter(row => row.event === 'inputUiHealthObserved')
      expect(health).toHaveLength(1800)
      expect(producer.snapshot().producerRowOversized).toBe(0)
      expect(health.every(row => encoder.encode(`${JSON.stringify(row)}\n`).length <= 1024)).toBe(true)
      expect(rows.reduce((sum, row) => sum + encoder.encode(`${JSON.stringify(row)}\n`).length, 0) / 30).toBeLessThanOrEqual(65536)
      expect(inputUiTrace.snapshot().coalesced).toBeGreaterThan(0)
      expect(JSON.stringify(rows)).not.toContain('private-device')
      if (process.env.PRODUCTION_TRACE_METRICS_PATH) {
        const file = process.env.PRODUCTION_TRACE_METRICS_PATH
        const metrics = JSON.parse(readFileSync(file, 'utf8'))
        metrics.input = { healthRecords: health.length, bytesPerMinute: rows.reduce((sum, row) => sum + encoder.encode(`${JSON.stringify(row)}\n`).length, 0) / 30, maxHealthRowBytes: Math.max(...health.map(row => encoder.encode(`${JSON.stringify(row)}\n`).length)), coalesced: inputUiTrace.snapshot().coalesced, producer: producer.snapshot() }
        writeFileSync(file, `${JSON.stringify(metrics, null, 2)}\n`)
      }
    }
    finally {
      inputUiTrace.scope('stream.diagnostics-menu-sheet', false, true)
      dispose()
    }
  })
})
