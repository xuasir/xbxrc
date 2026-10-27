import type { GameControls } from './preferences'
import type { ButtonKey } from './state'
import { BUTTON_KEYS } from './state'

export const BUTTON_LABELS: Record<ButtonKey, string> = {
  south: 'A',
  east: 'B',
  west: 'X',
  north: 'Y',
  l1: 'LB',
  r1: 'RB',
  l2: 'LT',
  r2: 'RT',
  l3: 'LS',
  r3: 'RS',
  view: 'View',
  menu: 'Menu',
  home: 'Xbox',
  dpadUp: '↑',
  dpadDown: '↓',
  dpadLeft: '←',
  dpadRight: '→',
}

export function assignGameButton(controls: GameControls, target: ButtonKey, source: ButtonKey): boolean {
  if (target === 'home' || source === 'home')
    return false
  const triggers: ButtonKey[] = ['l2', 'r2']
  if (triggers.includes(target) !== triggers.includes(source))
    return false
  const other = BUTTON_KEYS.find(key => controls.buttons[key] === source)!
  controls.buttons[other] = controls.buttons[target]
  controls.buttons[target] = source
  return true
}
