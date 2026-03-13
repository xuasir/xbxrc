import type { Direction } from './pathfinding'

export type TabLevel = 'primary' | 'secondary' | 'tertiary' | string | number

export interface NodeDef {
  neighbors?: Partial<Record<Direction, string>>
  tabLevel?: TabLevel
}
