import type { AppConfig, AppConfigGroups } from './types'

export function splitConfigGroups(config: AppConfig): AppConfigGroups {
  return {
    app: {
      locale: config.locale,
      fullscreen: config.fullscreen,
      background_keepalive: config.background_keepalive,
      use_vulkan: config.use_vulkan,
      debug: config.debug
    },
    auth: {
      use_msal: config.use_msal,
      force_region_ip: config.force_region_ip
    },
    streaming: {
      resolution: config.resolution,
      xhome_bitrate_mode: config.xhome_bitrate_mode,
      xhome_bitrate: config.xhome_bitrate,
      xcloud_bitrate_mode: config.xcloud_bitrate_mode,
      xcloud_bitrate: config.xcloud_bitrate,
      audio_bitrate_mode: config.audio_bitrate_mode,
      audio_bitrate: config.audio_bitrate,
      enable_audio_control: config.enable_audio_control,
      preferred_game_language: config.preferred_game_language,
      codec: config.codec,
      polling_rate: config.polling_rate,
      video_format: config.video_format,
      ipv6: config.ipv6,
      performance_style: config.performance_style,
      stream_runtime_mode: config.stream_runtime_mode,
      display_options: { ...config.display_options }
    },
    input: {
      vibration: config.vibration
    },
    xhome: {
      xhome_auto_connect_server_id: config.xhome_auto_connect_server_id,
      xhome_turn_fallback: config.xhome_turn_fallback,
      power_on: config.power_on,
      server_url: config.server_url,
      server_username: config.server_username,
      server_credential: config.server_credential
    }
  }
}
