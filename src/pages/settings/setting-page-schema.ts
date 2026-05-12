/**
 * 设置页展示 schema：与 CONFIG_GROUP_DEFINITIONS（后端分桶）解耦，仅描述导航与板块结构。
 */

/** 与 `rpc.config.getGroups()` / grouping.rs 返回的分组键一致 */
export type RpcConfigGroupKey = 'app' | 'streaming' | 'host' | 'xcloud' | 'input'

export type SettingPageKey
  = | 'general'
    | 'streamingExperience'
    | 'connectionHost'
    | 'inputDevices'
    | 'advancedDiagnostics'

export type SettingToolId = 'inputDebug' | 'gamepadMapping'

export type SettingActionId = 'expertReset' | 'unlockDangerZone'

export type SettingSchemaItemDef
  = | { kind: 'field', fieldKey: string }
    | { kind: 'tool', toolId: SettingToolId }
    | { kind: 'action', actionId: SettingActionId }
    | { kind: 'notice', noticeKey: string }
    | { kind: 'groupSummary', summaryId: string }

export interface SettingSchemaSectionDef {
  key: string
  /** i18n 完整键，例如 setting.pages.general.sectionAppearance */
  labelKey: string
  items: SettingSchemaItemDef[]
}

/** 每个配置字段所在 RPC 分组（与 src-tauri/src/mods/config/grouping.rs 对齐） */
export const CONFIG_FIELD_RPC_GROUP: Record<string, RpcConfigGroupKey> = {
  locale: 'app',
  theme: 'app',
  fullscreen: 'app',
  background_keepalive: 'app',
  use_vulkan: 'app',
  ui_haptics: 'app',
  ui_audio: 'app',
  debug: 'app',
  runtime_trace_mode: 'app',

  resolution: 'streaming',
  xhome_resolution: 'streaming',
  video_format: 'streaming',
  display_options: 'streaming',
  enable_audio_control: 'streaming',
  xhome_bitrate_mode: 'streaming',
  xhome_bitrate: 'streaming',
  xcloud_bitrate_mode: 'streaming',
  xcloud_bitrate: 'streaming',
  audio_bitrate_mode: 'streaming',
  audio_bitrate: 'streaming',
  performance_style: 'streaming',
  codec: 'streaming',
  force_region_ip: 'streaming',
  preferred_game_language: 'streaming',
  ipv6: 'streaming',
  stream_runtime_mode: 'streaming',
  xhome_turn_fallback: 'streaming',
  server_url: 'streaming',
  server_username: 'streaming',
  server_credential: 'streaming',

  polling_rate: 'input',
  vibration: 'input',
  vibration_strength: 'input',
}

export const SETTING_PAGE_ORDER: readonly SettingPageKey[] = [
  'general',
  'streamingExperience',
  'connectionHost',
  'inputDevices',
  'advancedDiagnostics',
] as const

export const SETTING_PAGE_LABEL_KEYS: Record<SettingPageKey, string> = {
  general: 'setting.pages.general.title',
  streamingExperience: 'setting.pages.streamingExperience.title',
  connectionHost: 'setting.pages.connectionHost.title',
  inputDevices: 'setting.pages.inputDevices.title',
  advancedDiagnostics: 'setting.pages.advancedDiagnostics.title',
}

const PAGE_GENERAL: SettingSchemaSectionDef[] = [
  {
    key: 'appearance',
    labelKey: 'setting.pages.general.sectionAppearance',
    items: [
      { kind: 'field', fieldKey: 'locale' },
      { kind: 'field', fieldKey: 'theme' },
      { kind: 'field', fieldKey: 'fullscreen' },
      { kind: 'field', fieldKey: 'ui_haptics' },
      { kind: 'field', fieldKey: 'ui_audio' },
      { kind: 'field', fieldKey: 'background_keepalive' },
    ],
  },
]

const PAGE_STREAMING: SettingSchemaSectionDef[] = [
  {
    key: 'shared',
    labelKey: 'setting.pages.streamingExperience.sectionShared',
    items: [
      { kind: 'field', fieldKey: 'video_format' },
      { kind: 'field', fieldKey: 'display_options' },
      { kind: 'field', fieldKey: 'enable_audio_control' },
      { kind: 'field', fieldKey: 'codec' },
      { kind: 'field', fieldKey: 'audio_bitrate_mode' },
      { kind: 'field', fieldKey: 'audio_bitrate' },
      { kind: 'field', fieldKey: 'performance_style' },
    ],
  },
  {
    key: 'console',
    labelKey: 'setting.pages.streamingExperience.sectionConsole',
    items: [
      { kind: 'field', fieldKey: 'xhome_resolution' },
      { kind: 'field', fieldKey: 'xhome_bitrate_mode' },
      { kind: 'field', fieldKey: 'xhome_bitrate' },
    ],
  },
  {
    key: 'cloud',
    labelKey: 'setting.pages.streamingExperience.sectionCloud',
    items: [
      { kind: 'field', fieldKey: 'resolution' },
      { kind: 'field', fieldKey: 'xcloud_bitrate_mode' },
      { kind: 'field', fieldKey: 'xcloud_bitrate' },
    ],
  },
]

const PAGE_CONNECTION: SettingSchemaSectionDef[] = [
  {
    key: 'general',
    labelKey: 'setting.pages.connectionHost.sectionGeneral',
    items: [
      { kind: 'field', fieldKey: 'preferred_game_language' },
      { kind: 'field', fieldKey: 'force_region_ip' },
      { kind: 'field', fieldKey: 'ipv6' },
    ],
  },
  {
    key: 'console',
    labelKey: 'setting.pages.connectionHost.sectionConsole',
    items: [
      { kind: 'field', fieldKey: 'xhome_turn_fallback' },
    ],
  },
]

const PAGE_INPUT: SettingSchemaSectionDef[] = [
  {
    key: 'controller',
    labelKey: 'setting.pages.inputDevices.sectionController',
    items: [
      { kind: 'field', fieldKey: 'polling_rate' },
      { kind: 'field', fieldKey: 'vibration' },
      { kind: 'field', fieldKey: 'vibration_strength' },
    ],
  },
  {
    key: 'tools',
    labelKey: 'setting.pages.inputDevices.sectionTools',
    items: [
      { kind: 'tool', toolId: 'inputDebug' },
      { kind: 'tool', toolId: 'gamepadMapping' },
    ],
  },
]

export function getAdvancedDiagnosticsSections(dangerUnlocked: boolean): SettingSchemaSectionDef[] {
  const base: SettingSchemaSectionDef[] = [
    {
      key: 'advanced',
      labelKey: 'setting.pages.advancedDiagnostics.sectionAdvanced',
      items: [
        { kind: 'field', fieldKey: 'stream_runtime_mode' },
        { kind: 'field', fieldKey: 'debug' },
        { kind: 'field', fieldKey: 'runtime_trace_mode' },
        { kind: 'field', fieldKey: 'use_vulkan' },
      ],
    },
  ]

  if (!dangerUnlocked) {
    base.push({
      key: 'danger',
      labelKey: 'setting.pages.advancedDiagnostics.sectionDanger',
      items: [
        { kind: 'notice', noticeKey: 'setting.pages.advancedDiagnostics.dangerIntro' },
        { kind: 'action', actionId: 'unlockDangerZone' },
      ],
    })
    return base
  }

  base.push({
    key: 'danger',
    labelKey: 'setting.pages.advancedDiagnostics.sectionDanger',
    items: [
      { kind: 'notice', noticeKey: 'setting.streaming.expert.riskHint' },
      { kind: 'field', fieldKey: 'server_url' },
      { kind: 'field', fieldKey: 'server_username' },
      { kind: 'field', fieldKey: 'server_credential' },
      { kind: 'action', actionId: 'expertReset' },
    ],
  })
  return base
}

const STATIC_PAGE_SECTIONS: Record<
  Exclude<SettingPageKey, 'advancedDiagnostics'>,
  SettingSchemaSectionDef[]
> = {
  general: PAGE_GENERAL,
  streamingExperience: PAGE_STREAMING,
  connectionHost: PAGE_CONNECTION,
  inputDevices: PAGE_INPUT,
}

export function getSectionsForPage(
  pageKey: SettingPageKey,
  dangerUnlocked: boolean,
): SettingSchemaSectionDef[] {
  if (pageKey === 'advancedDiagnostics') {
    return getAdvancedDiagnosticsSections(dangerUnlocked)
  }
  return STATIC_PAGE_SECTIONS[pageKey]
}

export function getConfigFieldValue(
  groups: Record<string, unknown> | null,
  fieldKey: string,
): unknown {
  if (groups === null) {
    return undefined
  }
  const groupKey = CONFIG_FIELD_RPC_GROUP[fieldKey]
  if (groupKey === undefined) {
    return undefined
  }
  const bucket = groups[groupKey]
  if (typeof bucket !== 'object' || bucket === null) {
    return undefined
  }
  return (bucket as Record<string, unknown>)[fieldKey]
}
