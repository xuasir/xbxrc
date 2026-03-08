export type ConfigBitrateMode = 'Auto' | 'Custom'

export interface DisplayOptions {
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

export interface AppConfig {
  locale: string
  use_msal: boolean
  fullscreen: boolean
  resolution: number
  xhome_bitrate_mode: ConfigBitrateMode
  xhome_bitrate: number
  xhome_turn_fallback: boolean
  xcloud_bitrate_mode: ConfigBitrateMode
  xcloud_bitrate: number
  audio_bitrate_mode: ConfigBitrateMode
  audio_bitrate: number
  enable_audio_control: boolean
  preferred_game_language: string
  force_region_ip: string
  codec: string
  polling_rate: number
  vibration: boolean
  power_on: boolean
  video_format: string
  ipv6: boolean
  performance_style: boolean
  stream_runtime_mode: 'webrtc-direct' | 'rust-owned'
  server_url: string
  server_username: string
  server_credential: string
  background_keepalive: boolean
  display_options: DisplayOptions
  use_vulkan: boolean
}

export type AppConfigKey = keyof AppConfig

export const APP_CONFIG_KEYS = [
  'locale',
  'use_msal',
  'fullscreen',
  'resolution',
  'xhome_bitrate_mode',
  'xhome_bitrate',
  'xhome_turn_fallback',
  'xcloud_bitrate_mode',
  'xcloud_bitrate',
  'audio_bitrate_mode',
  'audio_bitrate',
  'enable_audio_control',
  'preferred_game_language',
  'force_region_ip',
  'codec',
  'polling_rate',
  'vibration',
  'power_on',
  'video_format',
  'ipv6',
  'performance_style',
  'stream_runtime_mode',
  'server_url',
  'server_username',
  'server_credential',
  'background_keepalive',
  'display_options',
  'use_vulkan'
] as const satisfies readonly AppConfigKey[]

export type ConfigAppGroup = Pick<AppConfig, 'locale' | 'fullscreen' | 'background_keepalive' | 'use_vulkan'>
export type ConfigStreamingGroup = Pick<
  AppConfig,
  | 'resolution'
  | 'use_msal'
  | 'force_region_ip'
  | 'audio_bitrate_mode'
  | 'audio_bitrate'
  | 'enable_audio_control'
  | 'preferred_game_language'
  | 'codec'
  | 'video_format'
  | 'ipv6'
  | 'performance_style'
  | 'stream_runtime_mode'
  | 'display_options'
>
export type ConfigHostGroup = Pick<
  AppConfig,
  | 'xhome_bitrate_mode'
  | 'xhome_bitrate'
  | 'xhome_turn_fallback'
  | 'power_on'
  | 'server_url'
  | 'server_username'
  | 'server_credential'
>
export type ConfigXcloudGroup = Pick<AppConfig, 'xcloud_bitrate_mode' | 'xcloud_bitrate'>
export type ConfigInputGroup = Pick<AppConfig, 'polling_rate' | 'vibration'>

export interface AppConfigGroups {
  app: ConfigAppGroup
  streaming: ConfigStreamingGroup
  host: ConfigHostGroup
  xcloud: ConfigXcloudGroup
  input: ConfigInputGroup
}
