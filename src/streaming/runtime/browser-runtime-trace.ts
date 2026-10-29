import type { RuntimeTraceRecordEventParams } from '@shared/rpc/runtimeTrace'

type TraceProfile = 'off' | 'production' | 'dev'
export type BrowserTracePayload = Record<string, unknown> | (() => Record<string, unknown>)

const OBSERVATION_EVENTS = new Set([
  'browserWebRtcStatsObserved',
  'browserObservationConsumed',
  'renderTelemetryObserved',
  'renderCauseClassified',
  'recoveryCauseClassified',
  'panelStatsObserved',
  'browserMediaResourcesSnapshot',
])

/** 先过滤，再构造记录；异步 IPC 的在途数与待发送队列保持有界。 */
export function createBrowserRuntimeTrace(input: {
  write: (params: RuntimeTraceRecordEventParams) => Promise<unknown>
  context: () => { sessionId: string | null, runtimeGeneration: number, clientGeneration: number }
}) {
  let profile: TraceProfile = import.meta.env.DEV ? 'dev' : 'off'
  let inFlight = 0
  let dropped = 0
  let droppedEssential = 0
  const pending: RuntimeTraceRecordEventParams[] = []

  function drain(): void {
    while (inFlight < 4 && pending.length > 0) {
      const params = pending.shift()!
      inFlight++
      const finish = () => {
        inFlight--
        drain()
      }
      try {
        void input.write(params).then(finish, finish)
      }
      catch {
        finish()
      }
    }
  }

  return {
    configure(mode: unknown): void {
      const normalized = typeof mode === 'string' ? mode.trim().toLowerCase() : ''
      profile = !['production', 'dev', 'debug', 'verbose', 'trace'].includes(normalized)
        ? 'off'
        : import.meta.env.DEV && ['dev', 'debug', 'verbose', 'trace'].includes(normalized) ? 'dev' : 'production'
      if (profile === 'off')
        pending.length = 0
    },
    record(event: string, payload: BrowserTracePayload): void {
      const observation = OBSERVATION_EVENTS.has(event)
      if (profile === 'off' || (profile === 'production' && observation))
        return
      if (pending.length >= 32) {
        dropped++
        if (observation)
          return
        const index = pending.findIndex(entry => entry.importance === 'debug')
        if (index < 0)
          droppedEssential++
        pending.splice(Math.max(0, index), 1)
      }
      const { sessionId, runtimeGeneration, clientGeneration } = input.context()
      const value = typeof payload === 'function' ? payload() : payload
      pending.push({
        event,
        sessionId: typeof value.sessionId === 'string' ? value.sessionId : sessionId,
        importance: observation ? 'debug' : 'essential',
        dimension: 'frontend',
        payload: { clockDomain: 'monotonic', observedAtMs: performance.now(), runtimeGeneration, clientGeneration, ...value },
      })
      drain()
    },
    snapshot: () => ({ profile, inFlight, pending: pending.length, dropped, droppedEssential }),
  }
}
