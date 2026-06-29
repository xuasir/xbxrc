export type SettingFieldControl
  = | 'toggle'
    | 'singleSelect'
    | 'textInput'
    | 'numberInput'

export interface SettingSelectOptionDefinition {
  value: string | number
  label: string
  description?: string
}

export interface SettingFieldDefinition {
  label: string
  description?: string
  control: SettingFieldControl
  options?: readonly SettingSelectOptionDefinition[]
  input?: SettingFieldInputDefinition
}

export interface SettingFieldInputDefinition {
  min?: number
  max?: number
  step?: number
}

const BITRATE_MODE_OPTIONS = [
  { value: 'Auto', label: 'Auto', description: 'Automatic' },
  { value: 'Custom', label: 'Manual', description: 'Use limit below' },
] as const satisfies readonly SettingSelectOptionDefinition[]

const PREFERRED_GAME_LANGUAGE_OPTIONS = [
  { value: '', label: 'Default', description: 'Follow the platform default language' },
  { value: 'ar-SA', label: 'Arabic (Saudi Arabia)' },
  { value: 'cs-CZ', label: 'Czech' },
  { value: 'da-DK', label: 'Danish' },
  { value: 'de-DE', label: 'German' },
  { value: 'el-GR', label: 'Greek' },
  { value: 'en-GB', label: 'English (United Kingdom)' },
  { value: 'en-US', label: 'English (United States)' },
  { value: 'es-ES', label: 'Spanish (Spain)' },
  { value: 'es-MX', label: 'Spanish (Mexico)' },
  { value: 'fi-FI', label: 'Finnish' },
  { value: 'fr-FR', label: 'French' },
  { value: 'he-IL', label: 'Hebrew' },
  { value: 'hu-HU', label: 'Hungarian' },
  { value: 'it-IT', label: 'Italian' },
  { value: 'ja-JP', label: '日本語' },
  { value: 'ko-KR', label: 'Korean' },
  { value: 'nb-NO', label: 'Norwegian' },
  { value: 'nl-NL', label: 'Dutch' },
  { value: 'pl-PL', label: 'Polish' },
  { value: 'pt-BR', label: 'Portuguese (Brazil)' },
  { value: 'pt-PT', label: 'Portuguese (Portugal)' },
  { value: 'ru-RU', label: 'Russian' },
  { value: 'sk-SK', label: 'Slovak' },
  { value: 'sv-SE', label: 'Swedish' },
  { value: 'tr-TR', label: 'Turkish' },
  { value: 'zh-CN', label: '简体中文' },
  { value: 'zh-TW', label: '繁體中文' },
] as const satisfies readonly SettingSelectOptionDefinition[]

const FORCE_REGION_IP_OPTIONS = [
  { value: '', label: 'Default', description: 'Use the default regional routing strategy' },
  { value: '203.41.44.20', label: 'Australia' },
  { value: '200.221.11.101', label: 'Brazil' },
  { value: '194.25.0.68', label: 'Europe' },
  { value: '210.131.113.123', label: 'Japan' },
  { value: '168.126.63.1', label: 'Korea' },
  { value: '4.2.2.2', label: 'United States' },
  { value: '104.211.224.146', label: 'South India' },
  { value: '104.211.96.159', label: 'Central India' },
] as const satisfies readonly SettingSelectOptionDefinition[]

const CODEC_OPTIONS = [
  { value: '', label: 'Auto', description: 'Automatic' },
  { value: 'video/H264-64', label: 'H.264 High', description: 'Sharper' },
  { value: 'video/H264-4d', label: 'H.264 Main', description: 'Balanced' },
  { value: 'video/H264-42e', label: 'H.264 Constrained Baseline', description: 'More compatible' },
  { value: 'video/H264-420', label: 'H.264 Baseline', description: 'Most compatible' },
] as const satisfies readonly SettingSelectOptionDefinition[]

const POLLING_RATE_OPTIONS = [
  { value: 250, label: '250 HZ' },
  { value: 83.33, label: '83.33 HZ' },
  { value: 62.5, label: '62.5 HZ' },
  { value: 50, label: '50 HZ' },
  { value: 41.67, label: '41.67 HZ' },
  { value: 35.71, label: '35.71 HZ' },
  { value: 31.25, label: '31.25 HZ' },
  { value: 27.78, label: '27.78 HZ' },
  { value: 25, label: '25 HZ' },
  { value: 22.73, label: '22.73 HZ' },
  { value: 20.83, label: '20.83 HZ' },
  { value: 19.23, label: '19.23 HZ' },
  { value: 17.86, label: '17.86 HZ' },
  { value: 16.67, label: '16.67 HZ' },
] as const satisfies readonly SettingSelectOptionDefinition[]

const RUNTIME_TRACE_MODE_OPTIONS = [
  {
    value: 'off',
    label: 'Off',
    description: 'No logs',
  },
  {
    value: 'production',
    label: 'Production',
    description: 'Key events',
  },
  {
    value: 'dev',
    label: 'Dev',
    description: 'Detailed',
  },
] as const satisfies readonly SettingSelectOptionDefinition[]

const VIDEO_FORMAT_OPTIONS = [
  { value: '', label: 'Original aspect', description: 'Keep ratio' },
  { value: 'Stretch', label: 'Stretch to fill', description: 'Fill screen' },
  { value: 'Zoom', label: 'Crop and zoom', description: 'Crop edges' },
  { value: '16:10', label: 'Fixed 16:10', description: '16:10' },
  { value: '18:9', label: 'Fixed 18:9', description: '18:9' },
  { value: '21:9', label: 'Fixed 21:9', description: '21:9' },
  { value: '4:3', label: 'Fixed 4:3', description: '4:3' },
] as const satisfies readonly SettingSelectOptionDefinition[]

const DISPLAY_PRESET_OPTIONS = [
  { value: 'standard', label: 'Standard' },
  { value: 'clear', label: 'Clear' },
  { value: 'soft', label: 'Soft' },
] as const satisfies readonly SettingSelectOptionDefinition[]

export interface SettingSectionDefinition {
  key: string
  label: string
  keys: readonly string[]
}

export interface SettingGroupDefinition {
  label: string
  sections: readonly SettingSectionDefinition[]
}

/** 历史分组定义；设置页展示顺序见 `src/pages/settings/setting-page-schema.ts`。保留以供后端分桶文档对齐或其它调用方参考。 */
export const CONFIG_GROUP_DEFINITIONS: Record<string, SettingGroupDefinition> = {
  app: {
    label: 'APP',
    sections: [
      {
        key: 'appearance',
        label: 'Appearance',
        keys: ['locale', 'theme', 'fullscreen'],
      },
      {
        key: 'graphics',
        label: 'Graphics',
        keys: ['use_vulkan'],
      },
      {
        key: 'runtime',
        label: 'Runtime',
        keys: ['background_keepalive', 'debug', 'runtime_trace_mode'],
      },
      {
        key: 'navigation',
        label: 'Navigation',
        keys: ['ui_haptics', 'ui_audio'],
      },
    ],
  },
  auth: {
    label: 'AUTH',
    sections: [
      {
        key: 'region',
        label: 'Region',
        keys: ['force_region_ip'],
      },
    ],
  },
  streaming: {
    label: 'STREAMING',
    sections: [
      {
        key: 'default',
        label: 'Default',
        keys: [
          'resolution',
          'xhome_resolution',
          'preferred_game_language',
          'enable_audio_control',
          'video_format',
          'performance_style',
        ],
      },
      {
        key: 'advanced',
        label: 'Advanced',
        keys: [
          'force_region_ip',
          'ipv6',
          'codec',
          'xhome_bitrate_mode',
          'xhome_bitrate',
          'xcloud_bitrate_mode',
          'xcloud_bitrate',
          'audio_bitrate_mode',
          'audio_bitrate',
          'stream_runtime_mode',
          'xhome_turn_fallback',
          'display_options',
          'super_resolution_experimental',
        ],
      },
      {
        key: 'expert',
        label: 'Expert',
        keys: ['server_url', 'server_username', 'server_credential'],
      },
    ],
  },
  input: {
    label: 'INPUT',
    sections: [
      {
        key: 'controller',
        label: 'Controller',
        keys: ['polling_rate', 'vibration', 'vibration_strength'],
      },
    ],
  },
  xcloud: {
    label: 'XCLOUD',
    sections: [
      {
        key: 'connection',
        label: 'Connection',
        keys: [],
      },
    ],
  },
  host: {
    label: 'HOST',
    sections: [
      {
        key: 'connection',
        label: 'Connection',
        keys: [],
      },
    ],
  },
}

export const CONFIG_FIELD_DEFINITIONS: Record<string, SettingFieldDefinition> = {
  locale: {
    label: 'Locale',
    description: 'Language used by the current UI',
    control: 'singleSelect',
    options: [
      {
        value: 'en',
        label: 'English',
        description: 'Use English for the application interface',
      },
      {
        value: 'zh',
        label: '简体中文',
        description: '应用界面使用简体中文',
      },
    ],
  },
  theme: {
    label: 'Theme',
    control: 'singleSelect',
    options: [
      {
        value: 'dark',
        label: 'Dark',
      },
      {
        value: 'light',
        label: 'Light',
      },
    ],
  },
  fullscreen: {
    label: 'Launch in fullscreen',
    control: 'toggle',
  },
  resolution: {
    label: 'Cloud gaming max resolution',
    control: 'singleSelect',
    options: [
      { value: 1440, label: '1440p' },
      { value: 1081, label: 'Auto (1080p HQ)' },
      { value: 1080, label: '1080p' },
      { value: 720, label: '720p' },
    ],
  },
  xhome_resolution: {
    label: 'Console streaming max resolution',
    control: 'singleSelect',
    options: [
      { value: 1081, label: 'Auto (1080p HQ)' },
      { value: 1080, label: '1080p' },
      { value: 720, label: '720p' },
    ],
  },
  xhome_bitrate_mode: {
    label: 'Console video bitrate',
    control: 'singleSelect',
    options: BITRATE_MODE_OPTIONS,
  },
  xhome_bitrate: {
    label: 'Console bitrate limit',
    description: 'Mb/s',
    control: 'numberInput',
    input: {
      min: 0,
      max: 200,
      step: 1,
    },
  },
  xhome_turn_fallback: {
    label: 'Allow TURN relay for console streaming',
    description: 'Allow relay when direct connection fails.',
    control: 'toggle',
  },
  xcloud_bitrate_mode: {
    label: 'Cloud video bitrate',
    control: 'singleSelect',
    options: BITRATE_MODE_OPTIONS,
  },
  xcloud_bitrate: {
    label: 'Cloud bitrate limit',
    description: 'Mb/s',
    control: 'numberInput',
    input: {
      min: 0,
      max: 200,
      step: 1,
    },
  },
  audio_bitrate_mode: {
    label: 'Audio bitrate',
    control: 'singleSelect',
    options: BITRATE_MODE_OPTIONS,
  },
  audio_bitrate: {
    label: 'Audio bitrate limit',
    description: 'kb/s',
    control: 'numberInput',
    input: {
      min: 0,
      max: 512,
      step: 1,
    },
  },
  enable_audio_control: {
    label: 'In-stream volume control',
    control: 'toggle',
  },
  preferred_game_language: {
    label: 'Preferred game language',
    control: 'singleSelect',
    options: PREFERRED_GAME_LANGUAGE_OPTIONS,
  },
  force_region_ip: {
    label: 'Region routing override',
    control: 'singleSelect',
    options: FORCE_REGION_IP_OPTIONS,
  },
  codec: {
    label: 'Preferred video codec tier',
    control: 'singleSelect',
    options: CODEC_OPTIONS,
  },
  polling_rate: {
    label: 'Polling Rate',
    control: 'singleSelect',
    options: POLLING_RATE_OPTIONS,
  },
  vibration: {
    label: 'Vibration',
    control: 'toggle',
  },
  vibration_strength: {
    label: 'Vibration Strength',
    control: 'singleSelect',
    options: [
      {
        value: 'realistic',
        label: 'Realistic',
        description: 'Lighter',
      },
      {
        value: 'enhanced',
        label: 'Enhanced',
        description: 'Balanced',
      },
      {
        value: 'full',
        label: 'Full',
        description: 'Strongest',
      },
    ],
  },
  video_format: {
    label: 'Picture fit mode',
    control: 'singleSelect',
    options: VIDEO_FORMAT_OPTIONS,
  },
  ipv6: {
    label: 'Prefer IPv6',
    control: 'toggle',
  },
  performance_style: {
    label: 'Compact performance overlay',
    control: 'toggle',
  },
  stream_runtime_mode: {
    label: 'Streaming runtime',
    control: 'singleSelect',
    options: [
      {
        value: 'webrtc-direct',
        label: 'WebRTC Direct',
      },
      {
        value: 'rust-owned',
        label: 'Rust Owned',
      },
    ],
  },
  server_url: {
    label: 'Custom server URL',
    control: 'textInput',
  },
  server_username: {
    label: 'Custom server username',
    control: 'textInput',
  },
  server_credential: {
    label: 'Custom server credential',
    control: 'textInput',
  },
  background_keepalive: {
    label: 'Background keepalive',
    description: 'Keep the app session alive when the window moves to the background for faster return',
    control: 'toggle',
  },
  display_options: {
    label: 'Picture style',
    control: 'singleSelect',
    options: DISPLAY_PRESET_OPTIONS,
  },
  super_resolution_experimental: {
    label: 'Super Resolution (Experimental)',
    description: 'Browser FSR1 spatial upscale; may increase GPU load. Falls back to standard sharpening on failure.',
    control: 'toggle',
  },
  use_vulkan: {
    label: 'Use Vulkan rendering',
    description: 'Prefer the Vulkan rendering path; turn this off if your driver or platform is unstable with Vulkan',
    control: 'toggle',
  },
  ui_haptics: {
    label: 'UI haptic feedback',
    control: 'toggle',
  },
  ui_audio: {
    label: 'UI sound feedback',
    control: 'toggle',
  },
  debug: {
    label: 'Debug mode',
    control: 'toggle',
  },
  runtime_trace_mode: {
    label: 'Runtime trace logging',
    control: 'singleSelect',
    options: [...RUNTIME_TRACE_MODE_OPTIONS],
  },
}
