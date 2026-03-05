import type { StreamingTargetType } from '../../../../../shared/rpc/streaming'
import { PlayerClient } from '../../../player'
import type { RendererRuntimeConfig, TransportRuntimeConfig } from '../../../player'
import type { StreamConfigSnapshot } from '../../types'
import { normalizeDisplayOptions } from '../../utils'
import type { StreamRuntimeFactory } from '../contracts'
import { WebRtcDirectRuntime } from './WebRtcDirectRuntime'
import { createMainProcessRemoteSessionBridge } from './main-remote-session-bridge'

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

function createWebRtcDirectPlayerClient(input: {
  viewportElementId: string
  targetType: StreamingTargetType
  config: StreamConfigSnapshot
  audioVolume: number
}): PlayerClient {
  const displayOptions = normalizeDisplayOptions(input.config.display_options)

  return new PlayerClient({
    container: input.viewportElementId,
    input: {
      pollingRate: typeof input.config.polling_rate === 'number' ? input.config.polling_rate : 250,
      vibrationEnabled: input.config.vibration ?? true
    },
    audio: {
      volume: input.audioVolume,
      enableAudioControl: input.config.enable_audio_control === true
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
    }
  })
}

export const webRtcDirectRuntimeFactory: StreamRuntimeFactory = {
  supports(mode) {
    return mode === 'webrtc-direct'
  },
  async createRuntime(input) {
    return new WebRtcDirectRuntime(
      () =>
        createWebRtcDirectPlayerClient({
          viewportElementId: input.viewportElementId,
          targetType: input.targetType,
          config: input.config,
          audioVolume: input.audioVolume
        }),
      createMainProcessRemoteSessionBridge(),
      input.viewportElementId
    )
  }
}
