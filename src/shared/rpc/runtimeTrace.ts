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
}
