import { computed } from 'vue'
import type { RouteLocationNormalizedLoaded } from 'vue-router'
import type { StreamingTargetType } from '../../../../shared/rpc/streaming'
import type {
  StreamConfigSnapshot,
  StreamErrorKind,
  StreamRouteDescriptor,
  StreamingSession,
  TurnServerConfig
} from '../types'
import { extractQueueSeconds, normalizeErrorMessage } from '../utils'

export const STREAM_CONFIG_KEYS = [
  'enable_native_mouse_keyboard',
  'input_mousekeyboard_maping',
  'xhome_bitrate_mode',
  'xhome_bitrate',
  'xcloud_bitrate_mode',
  'xcloud_bitrate',
  'audio_bitrate_mode',
  'audio_bitrate',
  'enable_audio_control',
  'enable_audio_rumble',
  'audio_rumble_threshold',
  'resolution',
  'polling_rate',
  'vibration',
  'vibration_mode',
  'gamepad_kernal',
  'gamepad_mix',
  'gamepad_index',
  'dead_zone',
  'edge_compensation',
  'gamepad_maping',
  'force_trigger_rumble',
  'codec',
  'video_format',
  'display_options',
  'performance_style',
  'mouse_sensitive',
  'server_url',
  'server_username',
  'server_credential',
  'xhome_turn_fallback',
  'power_on'
] as const

interface StreamErrorInput {
  error: unknown
  t: (key: string, params?: Record<string, unknown>) => string
}

interface ResolveTurnServerInput {
  streamConfig: StreamConfigSnapshot
  useFallbackTurn: boolean
  targetType: StreamingTargetType
  fallbackTurnServer: TurnServerConfig | null
}

interface RetryFallbackInput extends ResolveTurnServerInput {
  fallbackRetryDone: boolean
}

interface WakeConsoleInput {
  targetType: StreamingTargetType
  streamConfig: StreamConfigSnapshot
  initialPowerState: string
  initialRemoteManagementEnabled: boolean
}

function resolveRouteDescriptor(route: RouteLocationNormalizedLoaded): StreamRouteDescriptor {
  const targetType = route.meta.streamTargetType === 'cloud' ? 'cloud' : 'home'
  const rawTargetId = route.params.targetId
  const targetId = typeof rawTargetId === 'string' ? rawTargetId : ''
  const rawName = route.query.name
  const displayName =
    typeof rawName === 'string' && rawName.trim() !== ''
      ? rawName.trim()
      : targetType === 'cloud'
        ? 'Xbox Cloud Gaming'
        : 'Xbox'

  return {
    targetType,
    targetId,
    displayName,
    exitRoute: targetType === 'cloud' ? '/xcloud' : '/xhome'
  }
}

/**
 * 路由相关状态集中在这里，避免远端会话层直接处理 route 细节。
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
  const initialRemoteManagementEnabled = computed(
    () => route.query.remoteManagementEnabled === '1'
  )

  return {
    targetType,
    targetId,
    displayName,
    exitRoute,
    initialPowerState,
    initialRemoteManagementEnabled
  }
}

export function resolveStreamError(input: StreamErrorInput): {
  kind: StreamErrorKind
  message: string
} {
  const message = normalizeErrorMessage(input.error)
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

export function shouldWakeConsole(input: WakeConsoleInput): boolean {
  return (
    input.targetType === 'home' &&
    input.streamConfig.power_on === true &&
    input.initialPowerState === 'ConnectedStandby' &&
    input.initialRemoteManagementEnabled
  )
}

export function resolveTurnServerConfig(input: ResolveTurnServerInput): TurnServerConfig | null {
  const config = input.streamConfig
  if (
    typeof config.server_url === 'string' &&
    config.server_url.length > 0 &&
    typeof config.server_username === 'string' &&
    config.server_username.length > 0 &&
    typeof config.server_credential === 'string' &&
    config.server_credential.length > 0
  ) {
    return {
      url: config.server_url,
      username: config.server_username,
      credential: config.server_credential
    }
  }

  if (input.useFallbackTurn && input.targetType === 'home') {
    return input.fallbackTurnServer
  }

  return null
}

export function canRetryWithFallbackTurn(input: RetryFallbackInput): boolean {
  return (
    input.targetType === 'home' &&
    input.useFallbackTurn === false &&
    input.fallbackRetryDone === false &&
    input.streamConfig.xhome_turn_fallback === true &&
    resolveTurnServerConfig(input) === null &&
    input.fallbackTurnServer !== null
  )
}

export function resolveQueuedStatusText(
  session: StreamingSession,
  t: (key: string, params?: Record<string, unknown>) => string
): string {
  const queueSeconds = extractQueueSeconds(session)
  return queueSeconds === null
    ? t('streamPage.status.waitingResources')
    : t('streamPage.status.waitingQueue', { seconds: queueSeconds })
}
