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
  disposition: 'enqueued' | 'buffered' | 'filtered' | 'queueFull' | 'rowOversized' | 'writerUnavailable'
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

export interface DiagnosticCapture {
  id: string
  traceRunId: string
  startedAtMs: number
  expiresAtMs: number
  endedAtMs: number | null
  state: 'active' | 'completed' | 'interrupted'
}
export interface DiagnosticIssue {
  id: string
  traceRunId: string
  source: string
  operation: string
  code: string
  firstAtMs: number
  lastAtMs: number
  fromMs: number
  toMs: number
  count: number
}
export interface DiagnosticSaveParams { issueId?: string, captureId?: string }
export interface DiagnosticSaveResult { saved: boolean, canceled: boolean }
export interface DiagnosticRecord {
  id: string
  kind: 'issue' | 'capture'
  traceRunId: string
  startedAtMs: number
  endedAtMs: number
  state: 'active' | 'completed' | 'interrupted'
  source: string
  count: number
  operation?: string
  code?: string
  available: boolean
  bytes: number
}
export interface RuntimeTraceStatus {
  traceRunId: string
  backend: Record<string, unknown>
  diagnostics: {
    effectiveMode: 'development' | 'production'
    lightweight: boolean
    recordingAvailable: boolean
    capture: DiagnosticCapture | null
    latestIssue: DiagnosticIssue | null
    records: DiagnosticRecord[]
    recordRetention: { maxRecords: number, maxBytes: number }
    bufferedRows: number
    bufferedBytes: number
    bufferedTotal: number
    contextDropped: number
    replayDropped: number
    healthPersistenceMs: number
  }
}

export interface RuntimeTraceExport { suggestedName: string, dataBase64: string, manifest: Record<string, unknown> }
