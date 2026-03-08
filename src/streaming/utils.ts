import type { StreamingAnswerPayload, StreamingQueueDetails } from '@shared/rpc/streaming'
import type { DisplayOptionsValue, StreamingSession } from './types'

export const DEFAULT_DISPLAY_OPTIONS: DisplayOptionsValue = {
  sharpness: 0,
  saturation: 100,
  contrast: 100,
  brightness: 100,
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms)
  })
}

export function extractAnswerSdp(payload: StreamingAnswerPayload): string {
  return payload.sdp
}

export function extractQueueSeconds(session: StreamingSession): number | null {
  const details = session?.queue?.details
  if (details === undefined) {
    return null
  }

  const seconds = (details as StreamingQueueDetails).estimatedTotalWaitTimeInSeconds
  return typeof seconds === 'number' && Number.isFinite(seconds) ? seconds : null
}

export function normalizeErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message
  }
  if (typeof error === 'string') {
    return error
  }
  try {
    return JSON.stringify(error)
  }
  catch {
    return 'unknown'
  }
}

export function normalizeDisplayOptions(value: unknown): DisplayOptionsValue {
  if (!isRecord(value)) {
    return { ...DEFAULT_DISPLAY_OPTIONS }
  }

  return {
    sharpness: toFiniteNumber(value.sharpness, DEFAULT_DISPLAY_OPTIONS.sharpness),
    saturation: toFiniteNumber(value.saturation, DEFAULT_DISPLAY_OPTIONS.saturation),
    contrast: toFiniteNumber(value.contrast, DEFAULT_DISPLAY_OPTIONS.contrast),
    brightness: toFiniteNumber(value.brightness, DEFAULT_DISPLAY_OPTIONS.brightness),
  }
}

export function isAspectRatioFormat(value: string | undefined): value is string {
  return typeof value === 'string' && value.includes(':')
}

function toFiniteNumber(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}
