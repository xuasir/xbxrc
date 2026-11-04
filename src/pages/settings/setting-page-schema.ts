/**
 * 设置页展示 schema：与 CONFIG_GROUP_DEFINITIONS（后端分桶）解耦，仅描述导航与板块结构。
 */

/** 与 `rpc.config.getGroups()` / grouping.rs 返回的分组键一致 */
export type RpcConfigGroupKey = 'app' | 'streaming' | 'host' | 'xcloud' | 'input'

export type SettingPageKey
  = | 'general'
    | 'streamingExperience'
    | 'connectionHost'
    | 'advancedDiagnostics'

export type SettingActionId = 'customTurn'

export type SettingEffect = 'instant' | 'nextConnection' | 'nextLaunch'

export type SettingSchemaItemDef
  = | { kind: 'field', fieldKey: string }
    | { kind: 'action', actionId: SettingActionId }
    | { kind: 'notice', noticeKey: string }
    | { kind: 'groupSummary', summaryId: string }

export interface SettingSchemaSectionDef {
  key: string
  /** i18n 完整键，例如 setting.pages.general.sectionAppearance */
  labelKey: string
  items: SettingSchemaItemDef[]
  collapsible?: boolean
}

/** 每个配置字段所在 RPC 分组（与 src-tauri/src/mods/config/grouping.rs 对齐） */
export const CONFIG_FIELD_RPC_GROUP: Record<string, RpcConfigGroupKey> = {
  locale: 'app',
  theme: 'app',
  fullscreen: 'app',
  background_keepalive: 'app',
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
  super_resolution_experimental: 'streaming',
  codec: 'streaming',
  force_region_ip: 'streaming',
  preferred_game_language: 'streaming',
  ipv6: 'streaming',
  xhome_turn_fallback: 'streaming',
  server_url: 'streaming',
  server_username: 'streaming',
  server_credential: 'streaming',

  vibration: 'input',
  vibration_strength: 'input',
}

export function getSettingEffect(fieldKey: string): SettingEffect {
  if (fieldKey === 'fullscreen')
    return 'nextLaunch'
  if (CONFIG_FIELD_RPC_GROUP[fieldKey] === 'streaming')
    return 'nextConnection'
  return 'instant'
}

export const SETTING_PAGE_ORDER: readonly SettingPageKey[] = [
  'general',
  'streamingExperience',
  'connectionHost',
  'advancedDiagnostics',
] as const

export const SETTING_PAGE_LABEL_KEYS: Record<SettingPageKey, string> = {
  general: 'setting.pages.general.title',
  streamingExperience: 'setting.pages.streamingExperience.title',
  connectionHost: 'setting.pages.connectionHost.title',
  advancedDiagnostics: 'setting.pages.advancedDiagnostics.title',
}

const PAGE_GENERAL: SettingSchemaSectionDef[] = [
  { key: 'appearance', labelKey: 'setting.pages.general.sectionAppearance', items: [
    { kind: 'field', fieldKey: 'locale' },
    { kind: 'field', fieldKey: 'theme' },
  ] },
  { key: 'startup', labelKey: 'setting.experience.sections.startup', items: [
    { kind: 'field', fieldKey: 'fullscreen' },
  ] },
  { key: 'feedback', labelKey: 'setting.experience.sections.feedback', items: [
    { kind: 'field', fieldKey: 'ui_audio' },
  ] },
]

const PAGE_STREAMING: SettingSchemaSectionDef[] = [
  { key: 'resolution', labelKey: 'setting.experience.sections.resolution', items: [
    { kind: 'field', fieldKey: 'xhome_resolution' },
    { kind: 'field', fieldKey: 'resolution' },
  ] },
  { key: 'picture', labelKey: 'setting.experience.sections.picture', items: [
    { kind: 'field', fieldKey: 'video_format' },
    { kind: 'field', fieldKey: 'display_options' },
    { kind: 'field', fieldKey: 'super_resolution_experimental' },
  ] },
  { key: 'sound', labelKey: 'setting.experience.sections.sound', items: [
    { kind: 'field', fieldKey: 'enable_audio_control' },
  ] },
  { key: 'advanced', labelKey: 'setting.experience.sections.streamingAdvanced', items: [
    { kind: 'field', fieldKey: 'codec' },
    { kind: 'field', fieldKey: 'xhome_bitrate_mode' },
    { kind: 'field', fieldKey: 'xhome_bitrate' },
    { kind: 'field', fieldKey: 'xcloud_bitrate_mode' },
    { kind: 'field', fieldKey: 'xcloud_bitrate' },
    { kind: 'field', fieldKey: 'audio_bitrate_mode' },
    { kind: 'field', fieldKey: 'audio_bitrate' },
  ] },
]

const PAGE_CONNECTION: SettingSchemaSectionDef[] = [
  { key: 'cloud', labelKey: 'setting.experience.sections.cloudPreferences', items: [
    { kind: 'field', fieldKey: 'force_region_ip' },
    { kind: 'field', fieldKey: 'preferred_game_language' },
  ] },
  { key: 'network', labelKey: 'setting.experience.sections.network', items: [
    { kind: 'field', fieldKey: 'ipv6' },
    { kind: 'field', fieldKey: 'xhome_turn_fallback' },
  ] },
  { key: 'advanced', labelKey: 'setting.experience.sections.connectionAdvanced', collapsible: true, items: [
    { kind: 'action', actionId: 'customTurn' },
  ] },
]

const PAGE_ADVANCED: SettingSchemaSectionDef[] = [
  { key: 'diagnostics', labelKey: 'setting.experience.sections.diagnostics', items: [
    { kind: 'field', fieldKey: 'runtime_trace_mode' },
    { kind: 'field', fieldKey: 'debug' },
  ] },
]

const PAGE_SECTIONS: Record<SettingPageKey, SettingSchemaSectionDef[]> = {
  general: PAGE_GENERAL,
  streamingExperience: PAGE_STREAMING,
  connectionHost: PAGE_CONNECTION,
  advancedDiagnostics: PAGE_ADVANCED,
}

export function getSectionsForPage(pageKey: SettingPageKey): SettingSchemaSectionDef[] {
  return PAGE_SECTIONS[pageKey]
}

/** 自动模式保留存储值，展示仅开放当前会消费的手动上限。 */
export function isSettingFieldVisible(groups: Record<string, unknown> | null, fieldKey: string): boolean {
  const modeKeys: Record<string, string> = { xhome_bitrate: 'xhome_bitrate_mode', xcloud_bitrate: 'xcloud_bitrate_mode', audio_bitrate: 'audio_bitrate_mode' }
  const modeKey = modeKeys[fieldKey]
  return !modeKey || getConfigFieldValue(groups, modeKey) === 'Custom'
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
