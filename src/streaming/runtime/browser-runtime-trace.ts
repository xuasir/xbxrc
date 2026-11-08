import type { RuntimeTraceAckResult, RuntimeTraceEventDescriptor, RuntimeTraceProducerPolicy, RuntimeTraceRecordEventParams } from '@shared/rpc/runtimeTrace'
import { productionTraceEvents, projectProductionTracePayload } from '../../shared/diagnostics/production-trace-contract'

type TraceProfile = 'off' | 'production' | 'dev'
export type BrowserTracePayload = Record<string, unknown> | (() => Record<string, unknown>)
export interface BrowserTraceRecordOptions { intervalManaged?: boolean, importance?: RuntimeTraceRecordEventParams['importance'], sessionId?: string | null }

const OBSERVATION_EVENTS = new Set([
  'browserWebRtcStatsObserved',
  'browserObservationConsumed',
  'renderTelemetryObserved',
  'renderCauseClassified',
  'recoveryCauseClassified',
  'panelStatsObserved',
  'browserMediaResourcesSnapshot',
])
// 配置读取失败时的安全默认值；正常启动使用 Rust 返回的有效 descriptor。
const DEFAULT_EVENTS = productionTraceEvents as RuntimeTraceEventDescriptor[]

/** 准入先于构造；字节预算、超时与 Rust 入队回执分别计数。 */
export function createBrowserRuntimeTrace(input: {
  write: (params: RuntimeTraceRecordEventParams) => Promise<unknown>
  context: () => { sessionId: string | null, runtimeGeneration: number, clientGeneration: number }
}) {
  let profile: TraceProfile = import.meta.env.DEV ? 'dev' : 'off'
  let descriptors = new Map(DEFAULT_EVENTS.map(event => [event.event, event]))
  let inFlight = 0
  let pendingBytes = 0
  let peakPendingBytes = 0
  let dropped = 0
  let droppedEssential = 0
  let coalesced = 0
  let profileFiltered = 0
  let ipcFailed = 0
  let ipcTimedOut = 0
  let payloadFailed = 0
  let producerRowOversized = 0
  let ackMissing = 0
  const backend = { enqueued: 0, filtered: 0, queueFull: 0, rowOversized: 0, writerUnavailable: 0 }
  const sampled = new Map<string, number>()
  const pending: Array<{ params: RuntimeTraceRecordEventParams, bytes: number }> = []
  const idleWaiters = new Set<() => void>()

  function notifyIdle(): void {
    if (pending.length === 0 && inFlight === 0)
      idleWaiters.forEach(resolve => resolve())
  }

  function remove(index: number, merge = false): void {
    const [entry] = pending.splice(index, 1)
    pendingBytes -= entry!.bytes
    if (merge)
      coalesced++
    else dropped++
    if (entry!.params.importance === 'essential')
      droppedEssential++
  }

  function drain(): void {
    while (inFlight < 4 && pending.length > 0) {
      const entry = pending.shift()!
      pendingBytes -= entry.bytes
      inFlight++
      let finished = false
      let timer: ReturnType<typeof setTimeout>
      const finish = (ack?: unknown, failure?: 'failed') => {
        if (finished)
          return
        finished = true
        clearTimeout(timer)
        if (failure === 'failed')
          ipcFailed++
        const disposition = (ack as Partial<RuntimeTraceAckResult> | undefined)?.disposition
        if (disposition && Object.hasOwn(backend, disposition))
          backend[disposition]++
        else if (!failure)
          ackMissing++
        inFlight--
        drain()
        notifyIdle()
      }
      // 超时说明证据状态；在真实 RPC 结束前保留 slot，防止慢桥接无限累积调用。
      timer = setTimeout(() => {
        ipcTimedOut++
      }, 2000)
      try {
        void input.write(entry.params).then(ack => finish(ack), () => finish(undefined, 'failed'))
      }
      catch { finish(undefined, 'failed') }
    }
  }

  const api = {
    configure(mode: unknown): void {
      const normalized = typeof mode === 'string' ? mode.trim().toLowerCase() : ''
      profile = ['off', 'none', '0'].includes(normalized)
        ? 'off'
        : ['dev', 'debug', 'verbose', 'trace'].includes(normalized) && import.meta.env.DEV
            ? 'dev'
            : ['production', 'prod', 'minimal', 'standard', 'dev', 'debug', 'verbose', 'trace'].includes(normalized) ? 'production' : 'off'
      if (profile === 'off') {
        profileFiltered += pending.length
        pending.length = 0
        pendingBytes = 0
        notifyIdle()
      }
      sampled.clear()
    },
    configurePolicy(policy: RuntimeTraceProducerPolicy): void {
      descriptors = new Map(policy.events.map(event => [event.event, event]))
      api.configure(policy.profile)
    },
    record(event: string, payload: BrowserTracePayload, options: BrowserTraceRecordOptions = {}): boolean {
      const observation = OBSERVATION_EVENTS.has(event)
      const playbackHealth = event === 'browserPlaybackHealthObserved'
      if (profile === 'off' || (profile === 'production' && observation) || (playbackHealth && profile !== 'production')) {
        profileFiltered++
        return false
      }
      const context = input.context()
      const observedAtMs = performance.now()
      const descriptor = descriptors.get(event)
      const key = `${event}:${context.sessionId}:${context.runtimeGeneration}:${context.clientGeneration}`
      if (!options.intervalManaged && descriptor?.intervalMs && sampled.has(key) && observedAtMs - sampled.get(key)! < descriptor.intervalMs) {
        coalesced++
        return false
      }
      const priority = options.importance ?? descriptor?.importance ?? (observation ? 'debug' : 'essential')
      if (pending.length >= 32) {
        if (playbackHealth) {
          const index = pending.findIndex(entry => entry.params.event === event)
          if (index >= 0) {
            remove(index, true)
          }
          else {
            dropped++
            return false
          }
        }
        else if (priority !== 'essential') {
          dropped++
          return false
        }
        else {
          remove(Math.max(0, pending.findIndex(entry => entry.params.importance !== 'essential')))
        }
      }
      let value: Record<string, unknown>
      try {
        value = typeof payload === 'function' ? payload() : payload
        if (profile === 'production')
          value = projectProductionTracePayload(event, value)
      }
      catch {
        payloadFailed++
        dropped++
        return false
      }
      const params: RuntimeTraceRecordEventParams = {
        event,
        sessionId: options.sessionId ?? (typeof value.sessionId === 'string' ? value.sessionId : context.sessionId),
        category: descriptor?.category,
        importance: priority,
        dimension: descriptor?.dimension ?? 'frontend',
        payload: { clockDomain: 'monotonic', observedAtMs, runtimeGeneration: context.runtimeGeneration, clientGeneration: context.clientGeneration, ...value },
      }
      let bytes: number
      try {
        bytes = new TextEncoder().encode(JSON.stringify(params)).byteLength
      }
      catch {
        payloadFailed++
        dropped++
        return false
      }
      if (bytes > (profile === 'production' ? descriptor?.maxRowBytes ?? 65536 : 65536) - 256) {
        producerRowOversized++
        dropped++
        return false
      }
      while (pendingBytes + bytes > 262144 && pending.length > 0) {
        const index = pending.findIndex(entry => entry.params.importance !== 'essential')
        if (index < 0 && priority !== 'essential') {
          dropped++
          return false
        }
        remove(Math.max(0, index))
      }
      sampled.set(key, observedAtMs)
      // 采样 slot 按事件/资源代次有界，旧 client 的 slot 随重建释放。
      if (sampled.size > 64)
        sampled.delete(sampled.keys().next().value!)
      pending.push({ params, bytes })
      pendingBytes += bytes
      peakPendingBytes = Math.max(peakPendingBytes, pendingBytes)
      drain()
      return true
    },
    flush(timeoutMs = 2000): Promise<boolean> {
      if (pending.length === 0 && inFlight === 0)
        return Promise.resolve(true)
      return new Promise((resolve) => {
        let timer: ReturnType<typeof setTimeout>
        const finish = () => {
          clearTimeout(timer)
          idleWaiters.delete(finish)
          resolve(true)
        }
        timer = setTimeout(() => {
          idleWaiters.delete(finish)
          resolve(false)
        }, timeoutMs)
        idleWaiters.add(finish)
      })
    },
    snapshot: () => ({ profile, inFlight, pending: pending.length, pendingBytes, peakPendingBytes, dropped, droppedEssential, coalesced, profileFiltered, ipcFailed, ipcTimedOut, payloadFailed, producerRowOversized, ackMissing, ...backend }),
  }
  return api
}
