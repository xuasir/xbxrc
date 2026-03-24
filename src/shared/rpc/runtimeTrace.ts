export interface RuntimeTraceRecordEventParams {
  event: string
  payload: unknown
  sessionId?: string | null
}

export interface RuntimeTraceAckResult {
  accepted: boolean
}
