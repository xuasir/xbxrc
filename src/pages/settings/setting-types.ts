import type {
  SettingFieldControl,
  SettingFieldInputDefinition,
  SettingSelectOptionDefinition,
} from '@shared/config/domain-definition'
import type { SettingTabKey } from '../../navigation/spatial-nav.constants'

export interface SettingTabNavItem {
  key: SettingTabKey
  label: string
  nodeId: string
  order: number
  upNeighborId: string
  downNeighborId?: string
  rightNeighborId?: string
}

export interface SettingRow {
  key: string
  label: string
  description?: string
  value: unknown
  valueText: string
  control: SettingFieldControl
  options?: readonly SettingSelectOptionDefinition[]
  input?: SettingFieldInputDefinition
  nodeId: string
  /** 展示「需重启」等生效提示 */
  needsRestart?: boolean
}

export interface SettingIndexedRow extends SettingRow {
  index: number
}

export interface SettingSectionEntryField {
  kind: 'field'
  row: SettingIndexedRow
}

export interface SettingSectionEntryTool {
  kind: 'tool'
  toolId: string
  nodeId: string
  label: string
  description?: string
  valueText: string
  index: number
}

export interface SettingSectionEntryAction {
  kind: 'action'
  actionId: string
  nodeId: string
  label: string
  variant: 'default' | 'danger'
  index: number
}

export interface SettingSectionEntryNotice {
  kind: 'notice'
  body: string
  index: number
}

export interface SettingSectionEntrySummary {
  kind: 'groupSummary'
  summaryId: string
  index: number
}

export type SettingSectionEntry
  = | SettingSectionEntryField
    | SettingSectionEntryTool
    | SettingSectionEntryAction
    | SettingSectionEntryNotice
    | SettingSectionEntrySummary

export interface SettingIndexedSection {
  key: string
  label: string
  entries: SettingSectionEntry[]
}

/** @deprecated 已由 schema entry 模型替代，保留别名避免大范围引用断裂 */
export type SettingSection = SettingIndexedSection
