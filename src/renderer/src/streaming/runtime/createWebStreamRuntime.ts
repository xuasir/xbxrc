import { MouseKeyboardConfig, StreamRuntimeClient } from './index'
import type { RendererRuntimeConfig, TransportRuntimeConfig } from './index'
import type { StreamingTargetType } from '../../../../shared/rpc/streaming'
import type { DisplayOptionsValue, StreamConfigSnapshot } from '../types'
import { DEFAULT_DISPLAY_OPTIONS, normalizeDisplayOptions } from '../utils'

type RuntimeMouseKeyboardMapping =
  NonNullable<ConstructorParameters<typeof MouseKeyboardConfig>[0]['keymapping']>

interface CreateWebStreamRuntimeInput {
  playerElementId: string
  targetType: StreamingTargetType
  config: StreamConfigSnapshot
  audioVolume: number
}

function toRendererFormat(videoFormat: string | undefined): RendererRuntimeConfig['format'] {
  if (videoFormat === 'Stretch') {
    return 'Stretch'
  }
  if (videoFormat === 'Zoom') {
    return 'Zoom'
  }
  return 'Contain'
}

function toCodecPreference(codec: string | undefined): TransportRuntimeConfig['codecPreference'] {
  if (typeof codec !== 'string' || codec.length === 0) {
    return undefined
  }

  if (codec.includes('H264')) {
    const [mimeType, profile] = codec.split('-')
    return {
      mimeType,
      profiles: profile !== undefined ? [profile] : []
    }
  }

  return {
    mimeType: codec,
    profiles: []
  }
}

function toCustomVideoBitrate(
  config: StreamConfigSnapshot,
  targetType: StreamingTargetType
): number {
  const bitrate = targetType === 'cloud' ? config.xcloud_bitrate : config.xhome_bitrate
  const mode = targetType === 'cloud' ? config.xcloud_bitrate_mode : config.xhome_bitrate_mode
  if (mode !== 'Custom' || typeof bitrate !== 'number' || bitrate <= 0) {
    return 0
  }
  return Math.round(bitrate * 1000)
}

function toCustomAudioBitrate(config: StreamConfigSnapshot): number {
  if (
    config.audio_bitrate_mode !== 'Custom' ||
    typeof config.audio_bitrate !== 'number' ||
    config.audio_bitrate <= 0
  ) {
    return 0
  }
  return Math.round(config.audio_bitrate * 1000)
}

function toMouseKeyboardConfig(
  mapping: StreamConfigSnapshot['input_mousekeyboard_maping']
): MouseKeyboardConfig {
  if (mapping === undefined) {
    return MouseKeyboardConfig.default()
  }

  const nextMapping: RuntimeMouseKeyboardMapping = {}
  for (const [key, value] of Object.entries(mapping)) {
    nextMapping[key] = value as RuntimeMouseKeyboardMapping[string]
  }
  return new MouseKeyboardConfig({ keymapping: nextMapping })
}

/**
 * Web runtime 的配置映射集中在这里，方便后续为移动端或 Rust runtime 增加并行实现。
 */
export function createWebStreamRuntime(input: CreateWebStreamRuntimeInput): {
  runtime: StreamRuntimeClient
  displayOptions: DisplayOptionsValue
} {
  const displayOptions = normalizeDisplayOptions(input.config.display_options)

  return {
    displayOptions,
    runtime: new StreamRuntimeClient({
      container: input.playerElementId,
      input: {
        touch: false,
        mouseKeyboard: input.config.enable_native_mouse_keyboard === true,
        legacyKeyboard: true,
        pollingRate:
          typeof input.config.polling_rate === 'number' ? input.config.polling_rate : 250,
        // 旧版网页播放器固定走 Web 内核；本地 runtime 先保持一致，避免 Native 路径引入额外前置依赖。
        gamepadKernel: 'Web',
        gamepadMix: input.config.gamepad_mix ?? false,
        gamepadIndex: input.config.gamepad_index ?? -1,
        vibrationEnabled: input.config.vibration ?? true,
        vibrationMode: 'Webview',
        gamepadDeadZone: input.config.dead_zone ?? 0.2,
        customGamepadMapping:
          (input.config.gamepad_maping as Record<string, string> | null | undefined) ?? null,
        forceTriggerRumble: input.config.force_trigger_rumble ?? '',
        edgeCompensation: input.config.edge_compensation ?? 0,
        mouseSensitivity: input.config.mouse_sensitive ?? 0.5
      },
      audio: {
        volume: input.audioVolume,
        enableAudioControl: input.config.enable_audio_control === true,
        enableAudioRumble: input.config.enable_audio_rumble === true,
        audioRumbleThreshold: input.config.audio_rumble_threshold ?? 0.15
      },
      renderer: {
        enabled: false,
        mode: 'webgl2',
        sharpness: displayOptions.sharpness,
        format: toRendererFormat(input.config.video_format)
      },
      transport: {
        codecPreference: toCodecPreference(input.config.codec),
        maxVideoBitrateKbps: toCustomVideoBitrate(input.config, input.targetType),
        maxAudioBitrateKbps: toCustomAudioBitrate(input.config),
        forceMonoAudio: false
      },
      mouseKeyboardConfig: toMouseKeyboardConfig(input.config.input_mousekeyboard_maping)
    })
  }
}

export function getDefaultStreamDisplayOptions(): DisplayOptionsValue {
  return { ...DEFAULT_DISPLAY_OPTIONS }
}
