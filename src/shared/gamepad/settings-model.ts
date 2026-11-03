import type { LogicalPadStateDto } from './contract'
import type { GameControls } from './preferences'
import type { ButtonKey } from './state'
import { BUTTON_KEYS, PadReleaseBarrier } from './state'

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

/** Menu/View 等释放后录入，让同时按住它们成为可靠的取消入口。 */
export function readGamepadCapture(previous: LogicalPadStateDto, next: LogicalPadStateDto): { kind: 'cancel' } | { kind: 'assign', source: ButtonKey } | undefined {
  if (next.buttons.menu > 0.5 && next.buttons.view > 0.5)
    return { kind: 'cancel' }
  const source = BUTTON_KEYS.find(key => key !== 'menu' && key !== 'view'
    && previous.buttons[key] <= 0.5 && next.buttons[key] > 0.5)
  ?? (['menu', 'view'] as const).find(key => previous.buttons[key] > 0.5 && next.buttons[key] <= 0.5)
  return source ? { kind: 'assign', source } : undefined
}

/** 录入从控制器当前帧建立释放屏障，避免打开录入的 A 被同帧再次消费。 */
export function createGamepadCaptureReader(initial: LogicalPadStateDto) {
  const barrier = new PadReleaseBarrier()
  barrier.capture(initial)
  let previous = barrier.apply(initial)
  return (state: LogicalPadStateDto) => {
    const next = barrier.apply(state)
    const action = readGamepadCapture(previous, next)
    previous = next
    return action
  }
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
