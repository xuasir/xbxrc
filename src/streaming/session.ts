import type {
  StreamingSessionError,
  StreamingStartupBoundedRetry,
  StreamingStartupError,
  StreamingStartupPhase,
  StreamingTargetType,
} from '@shared/rpc/streaming'
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
  runtimeLaunchState: StreamingSessionProgress['runtimeLaunchState']
  queueSeconds?: number
  queue?: {
    estimatedTotalWaitTimeInSeconds?: number
    estimatedAllocationTimeInSeconds?: number
    estimatedProvisioningTimeInSeconds?: number
  }
  errorCode?: string
  errorMessage?: string
  error?: StreamingSessionError
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
  'xhome_resolution',
  'codec',
  'video_format',
  'display_options',
  'ice_policy_enabled',
  'ice_policy_prefer_ipv6',
  'ice_policy_prefer_udp',
  'ice_policy_allow_tcp_fallback',
  'ice_policy_relay_bias',
  'ice_policy_enable_teredo_derivation',
  'ice_policy_enable_family_mismatch_gate',
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
  const controlConsoleId = computed(() => {
    const value = route.query.consoleId
    if (typeof value === 'string' && value.trim() !== '') {
      return value.trim()
    }
    return targetId.value
  })
  const initialPowerState = computed(() => {
    const value = route.query.powerState
    return typeof value === 'string' ? value : ''
  })
  const initialRemoteManagementEnabled = computed(() => route.query.remoteManagementEnabled === '1')

  return {
    targetType,
    targetId,
    controlConsoleId,
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
    attemptId: crypto.randomUUID(),
  })
}

export async function startRemoteStreamSessionWithAttempt(
  targetType: StreamingTargetType,
  targetId: string,
  attemptId: string,
) {
  return await rpc.streaming.startSession({
    targetType,
    targetId,
    attemptId,
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
    runtimeLaunchState: progress.runtimeLaunchState,
    queueSeconds: progress.queueSeconds,
    queue: progress.queue,
    errorCode: progress.errorCode,
    errorMessage: progress.errorMessage,
    error: progress.error,
    updatedAt: Date.now(),
  }
}

type StructuredSessionErrorLike = Pick<
  StreamingSessionError,
  'errorKind' | 'userMessageKey' | 'diagnosticSummary' | 'rawMessage' | 'retryable' | 'boundedRetry'
>

function resolveStructuredSessionError(
  error: StructuredSessionErrorLike,
  t: (key: string, params?: Record<string, unknown>) => string,
): {
  kind: StreamErrorKind
  message: string
  diagnosticSummary?: string
  boundedRetry?: StreamingStartupBoundedRetry
} {
  return {
    kind: 'startFailed',
    message: t(error.userMessageKey),
    diagnosticSummary: error.diagnosticSummary,
    boundedRetry: error.boundedRetry ?? undefined,
  }
}

export function resolveStreamError(input: StreamErrorInput): {
  kind: StreamErrorKind
  message: string
  diagnosticSummary?: string
  boundedRetry?: StreamingStartupBoundedRetry
} {
  const structuredStartupError = extractStructuredStartupError(input.error)
  if (structuredStartupError !== null) {
    return resolveStructuredSessionError(structuredStartupError, input.t)
  }

  const message = normalizeErrorMessage(input.error)
  if (message.startsWith('remoteConsoleNotReady:')) {
    const details = message.slice('remoteConsoleNotReady:'.length)
    return {
      kind: 'startFailed',
      message: input.t('streamPage.errors.remoteConsoleNotReady', {
        details,
      }),
      diagnosticSummary: details,
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

export function resolveProgressError(
  progress: StreamingSessionProgress,
  t: (key: string, params?: Record<string, unknown>) => string,
): {
  kind: StreamErrorKind
  message: string
  diagnosticSummary?: string
  boundedRetry?: StreamingStartupBoundedRetry
} {
  if (progress.error !== undefined && progress.error !== null) {
    return resolveStructuredSessionError(progress.error, t)
  }

  return {
    kind: 'startFailed',
    message: progress.errorMessage ?? t('streamPage.errors.connectionFailed'),
    diagnosticSummary: progress.errorMessage ?? undefined,
  }
}

export function resolveStartupPhaseStatusTextKey(
  phase: StreamingStartupPhase,
): string {
  switch (phase) {
    case 'resolvingContext':
      return 'streamPage.status.preparing'
    case 'creatingSession':
      return 'streamPage.status.creatingSession'
    case 'waitingSessionReady':
      return 'streamPage.status.waitingSession'
    case 'startingRuntime':
      return 'streamPage.status.startingPlayer'
    case 'ready':
      return 'streamPage.status.connectedWaitingMedia'
    case 'failed':
      return 'streamPage.errorTitle'
  }
}

export function resolveStartupPhasePrimaryStatusTextKey(
  phase: StreamingStartupPhase,
): string {
  switch (phase) {
    case 'resolvingContext':
      return 'streamPage.status.preparing'
    case 'creatingSession':
    case 'waitingSessionReady':
    case 'startingRuntime':
      return 'streamPage.status.startingStream'
    case 'ready':
      return 'streamPage.status.connectedWaitingMedia'
    case 'failed':
      return 'streamPage.errorTitle'
  }
}

export function createStartupAttemptId(): string {
  return crypto.randomUUID()
}

function extractStructuredStartupError(error: unknown): StreamingStartupError | null {
  if (!isRecord(error)) {
    return null
  }
  const details = isRecord(error.details) ? error.details : null
  if (details === null) {
    return null
  }
  if (
    typeof details.attemptId !== 'string'
    || typeof details.phase !== 'string'
    || typeof details.errorKind !== 'string'
    || typeof details.userMessageKey !== 'string'
    || typeof details.diagnosticSummary !== 'string'
    || typeof details.rawMessage !== 'string'
    || typeof details.retryable !== 'boolean'
  ) {
    return null
  }
  if (
    details.boundedRetry !== undefined
    && details.boundedRetry !== null
    && !isStreamingStartupBoundedRetry(details.boundedRetry)
  ) {
    return null
  }
  return details as unknown as StreamingStartupError
}

function isStreamingStartupBoundedRetry(value: unknown): value is StreamingStartupBoundedRetry {
  if (!isRecord(value)) {
    return false
  }
  return (
    typeof value.reason === 'string'
    && typeof value.status === 'string'
    && typeof value.retryCount === 'number'
    && typeof value.retryLimit === 'number'
  )
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
