export type ConfigBitrateMode = 'Auto' | 'Custom'

export interface DisplayOptions {
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

export type InputMouseKeyboardMapping = Record<string, string>
export type GamepadMapping = Record<string, unknown> | null

export interface AppConfig {
  locale: string
  use_msal: boolean
  fullscreen: boolean
  resolution: number
  xhome_auto_connect_server_id: string
  xhome_bitrate_mode: ConfigBitrateMode
  xhome_bitrate: number
  xhome_turn_fallback: boolean
  xcloud_bitrate_mode: ConfigBitrateMode
  xcloud_bitrate: number
  audio_bitrate_mode: ConfigBitrateMode
  audio_bitrate: number
  enable_audio_control: boolean
  enable_audio_rumble: boolean
  audio_rumble_threshold: number
  preferred_game_language: string
  force_region_ip: string
  codec: string
  polling_rate: number
  vibration: boolean
  vibration_mode: string
  gamepad_kernal: string
  gamepad_mix: boolean
  gamepad_index: number
  dead_zone: number
  edge_compensation: number
  force_trigger_rumble: '' | 'all' | 'left' | 'right'
  power_on: boolean
  video_format: string
  virtual_gamepad_opacity: number
  gamepad_maping: GamepadMapping
  ipv6: boolean
  enable_native_mouse_keyboard: boolean
  mouse_sensitive: number
  performance_style: boolean
  server_url: string
  server_username: string
  server_credential: string
  background_keepalive: boolean
  input_mousekeyboard_maping: InputMouseKeyboardMapping
  display_options: DisplayOptions
  use_vulkan: boolean
  debug: boolean
}

export type AppConfigKey = keyof AppConfig

export const APP_CONFIG_KEYS = [
  'locale',
  'use_msal',
  'fullscreen',
  'resolution',
  'xhome_auto_connect_server_id',
  'xhome_bitrate_mode',
  'xhome_bitrate',
  'xhome_turn_fallback',
  'xcloud_bitrate_mode',
  'xcloud_bitrate',
  'audio_bitrate_mode',
  'audio_bitrate',
  'enable_audio_control',
  'enable_audio_rumble',
  'audio_rumble_threshold',
  'preferred_game_language',
  'force_region_ip',
  'codec',
  'polling_rate',
  'vibration',
  'vibration_mode',
  'gamepad_kernal',
  'gamepad_mix',
  'gamepad_index',
  'dead_zone',
  'edge_compensation',
  'force_trigger_rumble',
  'power_on',
  'video_format',
  'virtual_gamepad_opacity',
  'gamepad_maping',
  'ipv6',
  'enable_native_mouse_keyboard',
  'mouse_sensitive',
  'performance_style',
  'server_url',
  'server_username',
  'server_credential',
  'background_keepalive',
  'input_mousekeyboard_maping',
  'display_options',
  'use_vulkan',
  'debug'
] as const satisfies readonly AppConfigKey[]

export type ConfigAppGroup = Pick<
  AppConfig,
  'locale' | 'fullscreen' | 'background_keepalive' | 'use_vulkan' | 'debug'
>
export type ConfigAuthGroup = Pick<AppConfig, 'use_msal' | 'force_region_ip'>
export type ConfigStreamingGroup = Pick<
  AppConfig,
  | 'resolution'
  | 'xhome_bitrate_mode'
  | 'xhome_bitrate'
  | 'xcloud_bitrate_mode'
  | 'xcloud_bitrate'
  | 'audio_bitrate_mode'
  | 'audio_bitrate'
  | 'enable_audio_control'
  | 'enable_audio_rumble'
  | 'audio_rumble_threshold'
  | 'preferred_game_language'
  | 'codec'
  | 'polling_rate'
  | 'video_format'
  | 'ipv6'
  | 'performance_style'
  | 'display_options'
>
export type ConfigInputGroup = Pick<
  AppConfig,
  | 'vibration'
  | 'vibration_mode'
  | 'gamepad_kernal'
  | 'gamepad_mix'
  | 'gamepad_index'
  | 'dead_zone'
  | 'edge_compensation'
  | 'force_trigger_rumble'
  | 'virtual_gamepad_opacity'
  | 'gamepad_maping'
  | 'enable_native_mouse_keyboard'
  | 'mouse_sensitive'
  | 'input_mousekeyboard_maping'
>
export type ConfigXhomeGroup = Pick<
  AppConfig,
  | 'xhome_auto_connect_server_id'
  | 'xhome_turn_fallback'
  | 'power_on'
  | 'server_url'
  | 'server_username'
  | 'server_credential'
>

export interface AppConfigGroups {
  app: ConfigAppGroup
  auth: ConfigAuthGroup
  streaming: ConfigStreamingGroup
  input: ConfigInputGroup
  xhome: ConfigXhomeGroup
}
