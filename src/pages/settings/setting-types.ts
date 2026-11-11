import type {
  SettingFieldControl,
  SettingFieldInputDefinition,
  SettingSelectOptionDefinition,
} from '@shared/config/domain-definition'
import type { SettingTabKey } from '../../navigation/spatial-nav.constants'
import type { SettingEffect } from './setting-page-schema'

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
  effect?: SettingEffect
}

export interface SettingIndexedRow extends SettingRow {
  index: number
}

export interface SettingSectionEntryField {
  kind: 'field'
  row: SettingIndexedRow
}

export interface SettingSectionEntryAction {
  kind: 'action'
  actionId: string
  nodeId: string
  label: string
  description?: string
  variant: 'default' | 'danger'
  value?: string
  index: number
}

export interface SettingSectionEntryNotice {
  kind: 'notice'
  label?: string
  value?: string
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
    | SettingSectionEntryAction
    | SettingSectionEntryNotice
    | SettingSectionEntrySummary

export interface SettingIndexedSection {
  key: string
  label: string
  entries: SettingSectionEntry[]
  collapsible?: boolean
}
