import type { StreamingTargetType } from '@shared/rpc/streaming'
import type { RouteLocationNormalizedLoaded } from 'vue-router'
import type {
  DisplayOptionsValue,
  StreamConfigSnapshot,
  StreamErrorKind,
  StreamingSessionProgress,
  StreamRouteDescriptor,
} from './types'
import { computed } from 'vue'
import { rpc } from '../services/rpc'
import { isRecord, normalizeErrorMessage } from './utils'

export type SessionUiPhase
  = | 'idle'
    | 'subscribing'
    | 'starting'
    | 'waiting'
    | 'connected'
    | 'failed'
    | 'closing'
    | 'closed'

export interface SessionHealthSnapshot {
  phase: StreamingSessionProgress['phase']
  retryCount: number
  queueSeconds?: number
  errorCode?: string
  errorMessage?: string
  updatedAt: number
}

interface StreamErrorInput {
  error: unknown
  t: (key: string, params?: Record<string, unknown>) => string
}

interface SessionProgressSubscriptionInput {
  getSessionId: () => string
  getEnabled: () => boolean
  loadProgress: (sessionId: string) => Promise<StreamingSessionProgress | null>
  onProgress: (progress: StreamingSessionProgress) => void
  onError: (error: unknown) => void
}

type BrowserInterval = number

const SESSION_PROGRESS_POLL_INTERVAL_MS = 1_000

export const STREAM_POLICY_CONFIG_KEYS = [
  'xhome_bitrate_mode',
  'xhome_bitrate',
  'xcloud_bitrate_mode',
  'xcloud_bitrate',
  'audio_bitrate_mode',
  'audio_bitrate',
  'enable_audio_control',
  'resolution',
  'codec',
  'video_format',
  'display_options',
  'server_url',
  'server_username',
  'server_credential',
  'xhome_turn_fallback',
  'power_on',
] as const

export const STREAM_VIEW_CONFIG_KEYS = ['performance_style'] as const

export const STREAM_CONFIG_KEYS = [
  ...STREAM_POLICY_CONFIG_KEYS,
  ...STREAM_VIEW_CONFIG_KEYS,
] as const

function resolveRouteDescriptor(route: RouteLocationNormalizedLoaded): StreamRouteDescriptor {
  const targetType = route.meta.streamTargetType === 'cloud' ? 'cloud' : 'home'
  const rawTargetId = route.params.targetId
  const targetId = typeof rawTargetId === 'string' ? rawTargetId : ''
  const rawName = route.query.name
  const displayName
    = typeof rawName === 'string' && rawName.trim() !== ''
      ? rawName.trim()
      : targetType === 'cloud'
        ? 'Xbox Cloud Gaming'
        : 'Xbox'

  return {
    targetType,
    targetId,
    displayName,
    exitRoute: targetType === 'cloud' ? '/xcloud' : '/xhome',
  }
}

/**
 * session 相关的 route 派生集中在这里，避免执行入口重新长成半个 controller。
 */
export function createStreamRouteState(route: RouteLocationNormalizedLoaded) {
  const routeDescriptor = computed(() => resolveRouteDescriptor(route))
  const targetType = computed<StreamingTargetType>(() => routeDescriptor.value.targetType)
  const targetId = computed(() => routeDescriptor.value.targetId)
  const displayName = computed(() => routeDescriptor.value.displayName)
  const exitRoute = computed(() => routeDescriptor.value.exitRoute)
  const initialPowerState = computed(() => {
    const value = route.query.powerState
    return typeof value === 'string' ? value : ''
  })
  const initialRemoteManagementEnabled = computed(() => route.query.remoteManagementEnabled === '1')

  return {
    targetType,
    targetId,
    displayName,
    exitRoute,
    initialPowerState,
    initialRemoteManagementEnabled,
  }
}

/**
 * session 启动与控制相关 RPC 统一收口，execution 入口只保留编排。
 */
export async function loadStreamConfigSnapshot(): Promise<StreamConfigSnapshot> {
  const config = await rpc.config.get({
    keys: [...STREAM_CONFIG_KEYS],
  })

  return isRecord(config) ? (config as StreamConfigSnapshot) : {}
}

export async function startRemoteStreamSession(targetType: StreamingTargetType, targetId: string) {
  return await rpc.streaming.startSession({
    targetType,
    targetId,
  })
}

export async function closeRemoteStreamSession(sessionId: string): Promise<void> {
  await rpc.streaming.closeSession({
    sessionId,
  })
}

export async function getRemoteSessionProgress(
  sessionId: string,
): Promise<StreamingSessionProgress | null> {
  return await rpc.streaming.getSessionProgress({
    sessionId,
  })
}

export async function powerOffRemoteConsole(consoleId: string): Promise<boolean> {
  const result = await rpc.data.powerOffConsole({
    consoleId,
  })
  return result.accepted
}

export async function sendTextToRemoteConsole(consoleId: string, text: string): Promise<boolean> {
  const result = await rpc.data.sendTextToConsole({
    consoleId,
    text,
  })
  return result.accepted
}

export async function persistStreamDisplayOptions(
  optionsValue: DisplayOptionsValue,
): Promise<void> {
  await rpc.config.set({
    patch: {
      display_options: optionsValue,
    },
  })
}

/**
 * 后端 progress phase 到页面 phase 的映射统一定义，避免 UI 继续分散判断。
 */
export function mapProgressToSessionUiPhase(progress: StreamingSessionProgress): SessionUiPhase {
  switch (progress.phase) {
    case 'creating':
      return 'starting'
    case 'waitingSessionReady':
    case 'runtimeStarting':
    case 'sessionReady':
    case 'recovering':
      return 'waiting'
    case 'closing':
      return 'closing'
    case 'closed':
      return 'closed'
    case 'failed':
      return 'failed'
    default:
      return 'waiting'
  }
}

/**
 * UI 只保留真正会展示的 session 健康字段，避免把后端 progress 全量扩散出去。
 */
export function buildSessionHealthSnapshot(
  progress: StreamingSessionProgress,
): SessionHealthSnapshot {
  return {
    phase: progress.phase,
    retryCount: progress.retryCount,
    queueSeconds: progress.queueSeconds,
    errorCode: progress.errorCode,
    errorMessage: progress.errorMessage,
    updatedAt: Date.now(),
  }
}

export function resolveStreamError(input: StreamErrorInput): {
  kind: StreamErrorKind
  message: string
} {
  const message = normalizeErrorMessage(input.error)
  if (message.startsWith('remoteConsoleNotReady:')) {
    const details = message.slice('remoteConsoleNotReady:'.length)
    return {
      kind: 'startFailed',
      message: input.t('streamPage.errors.remoteConsoleNotReady', {
        details,
      }),
    }
  }
  if (message === 'invalidAnswer') {
    return { kind: 'invalidAnswer', message: input.t('streamPage.errors.invalidAnswer') }
  }
  if (message === 'invalidOffer') {
    return { kind: 'invalidOffer', message: input.t('streamPage.errors.invalidOffer') }
  }
  if (message === 'sessionMissing') {
    return { kind: 'sessionMissing', message: input.t('streamPage.errors.sessionMissing') }
  }
  if (message === 'connectionFailed' || message === 'createOfferTimeout') {
    return { kind: 'connectionFailed', message: input.t('streamPage.errors.connectionFailed') }
  }
  if (message === 'connectionClosed') {
    return { kind: 'connectionClosed', message: input.t('streamPage.errors.connectionClosed') }
  }
  if (message === 'targetMissing') {
    return { kind: 'targetMissing', message: input.t('streamPage.errors.targetMissing') }
  }
  if (message === 'unknown') {
    return { kind: 'unknown', message: input.t('streamPage.errors.unknown') }
  }
  return { kind: 'unknown', message }
}

/**
 * UI 侧 progress 订阅只负责轮询和回调，不承担策略语义。
 */
export function createSessionProgressSubscription(input: SessionProgressSubscriptionInput) {
  let timer: BrowserInterval | null = null
  let polling = false
  let subscriptionToken = 0

  function stop(): void {
    subscriptionToken += 1
    if (timer !== null) {
      window.clearInterval(timer)
      timer = null
    }
  }

  async function pollOnce(token: number): Promise<void> {
    if (!input.getEnabled() || token !== subscriptionToken || polling) {
      return
    }

    const sessionId = input.getSessionId()
    if (sessionId === '') {
      return
    }

    polling = true
    try {
      const progress = await input.loadProgress(sessionId)
      if (
        token !== subscriptionToken
        || input.getSessionId() !== sessionId
        || progress === null
      ) {
        return
      }
      input.onProgress(progress)
    }
    catch (error) {
      if (token !== subscriptionToken) {
        return
      }
      input.onError(error)
    }
    finally {
      polling = false
    }
  }

  function start(): void {
    stop()
    if (!input.getEnabled()) {
      return
    }

    const token = subscriptionToken
    // 启动订阅时先拉一次，减少首屏状态抖动。
    void pollOnce(token)
    timer = window.setInterval(() => {
      void pollOnce(token)
    }, SESSION_PROGRESS_POLL_INTERVAL_MS)
  }

  return {
    start,
    stop,
  }
}
