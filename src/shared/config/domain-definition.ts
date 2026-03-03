export type SettingFieldControl =
  | 'toggle'
  | 'singleSelect'
  | 'textInput'
  | 'numberInput'
  | 'displayOptions'

export interface SettingSelectOptionDefinition {
  value: string | number
  label: string
  description?: string
  meta?: string
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
  { value: 'Auto', label: 'Auto', description: 'Use automatic bitrate selection' },
  { value: 'Custom', label: 'Custom', description: 'Use a custom bitrate value below' }
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
  { value: 'zh-TW', label: '繁體中文' }
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
  { value: '104.211.96.159', label: 'Central India' }
] as const satisfies readonly SettingSelectOptionDefinition[]

const CODEC_OPTIONS = [
  { value: '', label: 'Auto', description: 'Automatically select the most suitable codec' },
  { value: 'video/H264-4d', label: 'H264-High' },
  { value: 'video/H264-42e', label: 'H264-Medium' },
  { value: 'video/H264-420', label: 'H264-Low' }
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
  { value: 16.67, label: '16.67 HZ' }
] as const satisfies readonly SettingSelectOptionDefinition[]

const VIDEO_FORMAT_OPTIONS = [
  { value: '', label: 'Aspect Ratio', description: 'Keep the original stream aspect ratio' },
  { value: 'Stretch', label: 'Stretch' },
  { value: 'Zoom', label: 'Zoom' },
  { value: '16:10', label: '16:10' },
  { value: '18:9', label: '18:9' },
  { value: '21:9', label: '21:9' },
  { value: '4:3', label: '4:3' }
] as const satisfies readonly SettingSelectOptionDefinition[]

const GAMEPAD_INDEX_OPTIONS = [
  { value: -1, label: 'Auto', description: 'Automatically select the active controller' },
  { value: 0, label: '1' },
  { value: 1, label: '2' },
  { value: 2, label: '3' },
  { value: 3, label: '4' }
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

// 配置页严格复用 config domain 的 tab/section/字段顺序，避免前端自行发明结构
export const CONFIG_GROUP_DEFINITIONS: Record<string, SettingGroupDefinition> = {
  app: {
    label: 'APP',
    sections: [
      {
        key: 'appearance',
        label: 'Appearance',
        keys: ['locale', 'fullscreen']
      },
      {
        key: 'graphics',
        label: 'Graphics',
        keys: ['use_vulkan']
      },
      {
        key: 'runtime',
        label: 'Runtime',
        keys: ['background_keepalive', 'debug']
      }
    ]
  },
  auth: {
    label: 'AUTH',
    sections: [
      {
        key: 'provider',
        label: 'Provider',
        keys: ['use_msal']
      },
      {
        key: 'region',
        label: 'Region',
        keys: ['force_region_ip']
      }
    ]
  },
  streaming: {
    label: 'STREAMING',
    sections: [
      {
        key: 'video',
        label: 'Video',
        keys: [
          'resolution',
          'codec',
          'video_format',
          'display_options',
          'performance_style'
        ]
      },
      {
        key: 'bitrate',
        label: 'Bitrate',
        keys: [
          'xhome_bitrate_mode',
          'xhome_bitrate',
          'xcloud_bitrate_mode',
          'xcloud_bitrate',
          'audio_bitrate_mode',
          'audio_bitrate'
        ]
      },
      {
        key: 'audio',
        label: 'Audio',
        keys: ['enable_audio_control', 'enable_audio_rumble', 'audio_rumble_threshold']
      },
      {
        key: 'session',
        label: 'Session',
        keys: ['preferred_game_language', 'polling_rate', 'ipv6']
      }
    ]
  },
  input: {
    label: 'INPUT',
    sections: [
      {
        key: 'controller',
        label: 'Controller',
        keys: [
          'vibration',
          'vibration_mode',
          'gamepad_kernal',
          'gamepad_mix',
          'gamepad_index',
          'force_trigger_rumble'
        ]
      },
      {
        key: 'sticks',
        label: 'Sticks',
        keys: ['dead_zone', 'edge_compensation']
      },
      {
        key: 'virtualController',
        label: 'Virtual Controller',
        keys: ['virtual_gamepad_opacity']
      },
      {
        key: 'mouseKeyboard',
        label: 'Mouse & Keyboard',
        keys: ['enable_native_mouse_keyboard', 'mouse_sensitive']
      }
    ]
  },
  xhome: {
    label: 'XHOME',
    sections: [
      {
        key: 'connection',
        label: 'Connection',
        keys: ['xhome_auto_connect_server_id', 'xhome_turn_fallback', 'power_on']
      },
      {
        key: 'relay',
        label: 'Relay',
        keys: ['server_url', 'server_username', 'server_credential']
      }
    ]
  }
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
        description: 'Use English for the application interface'
      },
      {
        value: 'zh',
        label: '简体中文',
        description: '应用界面使用简体中文'
      }
    ]
  },
  use_msal: {
    label: 'Use MSAL',
    description: 'Switch between MSAL and XAL authentication',
    control: 'toggle'
  },
  fullscreen: {
    label: 'Fullscreen',
    description: 'Launch and display in fullscreen mode',
    control: 'toggle'
  },
  resolution: {
    label: 'Resolution',
    description:
      'All resolutions listed in this section represent maximum values as specified for each option; therefore, "1080p" denotes support for up to 1080p resolution.',
    control: 'singleSelect',
    options: [
      {
        value: 1081,
        label: 'Auto (1440p Max Quality)',
        description: "The recommended setting for the user's device and subscription tier"
      },
      { value: 1080, label: '1080p', meta: '7 GB/hr' },
      { value: 720, label: '720p', meta: '3 GB/hr' }
    ]
  },
  xhome_auto_connect_server_id: {
    label: 'Auto Connect Host',
    description: 'xHome host ID for automatic connection',
    control: 'textInput'
  },
  xhome_bitrate_mode: {
    label: 'xHome Bitrate Mode',
    description: 'Select automatic or manual bitrate for xHome',
    control: 'singleSelect',
    options: BITRATE_MODE_OPTIONS
  },
  xhome_bitrate: {
    label: 'xHome Bitrate',
    description: 'Target bitrate for xHome streaming in Mb/s',
    control: 'numberInput',
    input: {
      min: 0,
      max: 200,
      step: 1
    }
  },
  xhome_turn_fallback: {
    label: 'xHome TURN Fallback',
    description: 'Enable TURN fallback for xHome connectivity',
    control: 'toggle'
  },
  xcloud_bitrate_mode: {
    label: 'xCloud Bitrate Mode',
    description: 'Select automatic or manual bitrate for xCloud',
    control: 'singleSelect',
    options: BITRATE_MODE_OPTIONS
  },
  xcloud_bitrate: {
    label: 'xCloud Bitrate',
    description: 'Target bitrate for xCloud streaming in Mb/s',
    control: 'numberInput',
    input: {
      min: 0,
      max: 200,
      step: 1
    }
  },
  audio_bitrate_mode: {
    label: 'Audio Bitrate Mode',
    description: 'Select automatic or manual audio bitrate',
    control: 'singleSelect',
    options: BITRATE_MODE_OPTIONS
  },
  audio_bitrate: {
    label: 'Audio Bitrate',
    description: 'Target audio bitrate in Mb/s',
    control: 'numberInput',
    input: {
      min: 0,
      max: 200,
      step: 1
    }
  },
  enable_audio_control: {
    label: 'Audio Control',
    description: 'Enable in-stream audio volume control',
    control: 'toggle'
  },
  enable_audio_rumble: {
    label: 'Audio Rumble',
    description: 'Drive controller rumble from audio feedback',
    control: 'toggle'
  },
  audio_rumble_threshold: {
    label: 'Audio Rumble Threshold',
    description: 'Threshold for triggering audio-driven rumble',
    control: 'numberInput',
    input: {
      min: 0.01,
      max: 0.5,
      step: 0.01
    }
  },
  preferred_game_language: {
    label: 'Preferred Game Language',
    description: 'Language preference passed to the game session',
    control: 'singleSelect',
    options: PREFERRED_GAME_LANGUAGE_OPTIONS
  },
  force_region_ip: {
    label: 'Force Region IP',
    description: 'Override network region selection with a specific IP',
    control: 'singleSelect',
    options: FORCE_REGION_IP_OPTIONS
  },
  codec: {
    label: 'Codec',
    description: 'Preferred video codec',
    control: 'singleSelect',
    options: CODEC_OPTIONS
  },
  polling_rate: {
    label: 'Polling Rate',
    description: 'Controller polling rate in Hz',
    control: 'singleSelect',
    options: POLLING_RATE_OPTIONS
  },
  vibration: {
    label: 'Vibration',
    description: 'Enable controller vibration output',
    control: 'toggle'
  },
  vibration_mode: {
    label: 'Vibration Mode',
    description: 'Controller vibration implementation mode',
    control: 'textInput'
  },
  gamepad_kernal: {
    label: 'Gamepad Kernel',
    description: 'Underlying gamepad kernel mode',
    control: 'textInput'
  },
  gamepad_mix: {
    label: 'Gamepad Mixed Input',
    description: 'Allow mixed controller input handling',
    control: 'toggle'
  },
  gamepad_index: {
    label: 'Gamepad Index',
    description: 'Controller index, -1 means automatic selection',
    control: 'singleSelect',
    options: GAMEPAD_INDEX_OPTIONS
  },
  dead_zone: {
    label: 'Dead Zone',
    description: 'Analog stick dead zone',
    control: 'numberInput',
    input: {
      min: 0.1,
      max: 0.9,
      step: 0.01
    }
  },
  edge_compensation: {
    label: 'Edge Compensation',
    description: 'Analog stick edge compensation',
    control: 'numberInput',
    input: {
      min: 0,
      max: 0.2,
      step: 0.01
    }
  },
  force_trigger_rumble: {
    label: 'Trigger Rumble',
    description: 'Choose which trigger receives rumble feedback',
    control: 'singleSelect',
    options: [
      { value: '', label: 'Off', description: 'Disable trigger rumble override' },
      { value: 'all', label: 'All Triggers', description: 'Apply rumble to both triggers' },
      { value: 'left', label: 'Left Trigger', description: 'Apply rumble to the left trigger' },
      {
        value: 'right',
        label: 'Right Trigger',
        description: 'Apply rumble to the right trigger'
      }
    ]
  },
  power_on: {
    label: 'Power On',
    description: 'Wake the host automatically before streaming',
    control: 'toggle'
  },
  video_format: {
    label: 'Video Format',
    description: 'Preferred video display format',
    control: 'singleSelect',
    options: VIDEO_FORMAT_OPTIONS
  },
  virtual_gamepad_opacity: {
    label: 'Virtual Gamepad Opacity',
    description: 'Opacity of the on-screen virtual gamepad',
    control: 'numberInput',
    input: {
      min: 0,
      max: 1,
      step: 0.05
    }
  },
  ipv6: {
    label: 'Prefer IPv6',
    description: 'Prefer IPv6 candidates during streaming',
    control: 'toggle'
  },
  enable_native_mouse_keyboard: {
    label: 'Native Mouse Keyboard',
    description: 'Enable native mouse and keyboard input',
    control: 'toggle'
  },
  mouse_sensitive: {
    label: 'Mouse Sensitivity',
    description: 'Sensitivity multiplier for mouse input',
    control: 'numberInput',
    input: {
      min: 0.1,
      max: 5,
      step: 0.1
    }
  },
  performance_style: {
    label: 'Performance Style',
    description: 'Switch performance overlay presentation style',
    control: 'toggle'
  },
  server_url: {
    label: 'Server URL',
    description: 'Custom relay or self-hosted server URL',
    control: 'textInput'
  },
  server_username: {
    label: 'Server Username',
    description: 'Username for the custom server',
    control: 'textInput'
  },
  server_credential: {
    label: 'Server Credential',
    description: 'Credential for the custom server',
    control: 'textInput'
  },
  background_keepalive: {
    label: 'Background Keepalive',
    description: 'Keep the application session alive in background',
    control: 'toggle'
  },
  display_options: {
    label: 'Display Options',
    description: 'Sharpness, saturation, contrast and brightness',
    control: 'displayOptions'
  },
  use_vulkan: {
    label: 'Use Vulkan',
    description: 'Enable the Vulkan rendering path',
    control: 'toggle'
  },
  debug: {
    label: 'Debug',
    description: 'Enable debug mode',
    control: 'toggle'
  }
}
