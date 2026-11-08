export interface RuntimeTraceRecordEventParams {
  event: string
  payload: unknown
  sessionId?: string | null
  category?: 'event' | 'decision' | 'state' | 'snapshot' | 'log'
  dimension?:
    | 'core'
    | 'lifecycle'
    | 'network'
    | 'recovery'
    | 'media_supply'
    | 'presentation'
    | 'input'
    | 'native_video'
    | 'frontend'
    | 'engine_log'
  importance?: 'essential' | 'key' | 'debug' | 'raw'
}

export interface RuntimeTraceAckResult {
  accepted: boolean
  disposition: 'enqueued' | 'filtered' | 'queueFull' | 'rowOversized' | 'writerUnavailable'
}

export interface RuntimeTraceEventDescriptor {
  event: string
  category: NonNullable<RuntimeTraceRecordEventParams['category']>
  dimension: NonNullable<RuntimeTraceRecordEventParams['dimension']>
  importance: NonNullable<RuntimeTraceRecordEventParams['importance']>
  intervalMs: number
  maxRowBytes: number
}

export interface RuntimeTraceProducerPolicy {
  profile: 'off' | 'production' | 'dev'
  productionContractVersion: number
  generation?: number
  budget: { maxInFlight: number, maxPendingRows: number, maxPendingBytes: number, normalBytesPerMinute: number, inputBytesPerMinute: number, anomalyBytesPerMinute: number }
  events: RuntimeTraceEventDescriptor[]
}

export interface RuntimeTraceFlushResult {
  status: 'flushed' | 'off' | 'timeout' | 'ioError' | 'queueFull' | 'writerUnavailable'
  backend?: Record<string, unknown>
}

export interface RuntimeTraceStatus { traceRunId: string, backend: Record<string, unknown> }

export interface RuntimeTraceExport { suggestedName: string, dataBase64: string, manifest: Record<string, unknown> }
