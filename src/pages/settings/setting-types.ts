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
}

export interface SettingIndexedRow extends SettingRow {
  index: number
}

export interface SettingSection {
  key: string
  label: string
  rows: SettingRow[]
}

export interface SettingIndexedSection {
  key: string
  label: string
  rows: SettingIndexedRow[]
}

