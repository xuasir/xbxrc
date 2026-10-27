import type { LogicalButtonsStateDto, LogicalPadStateDto } from './contract'

export const BUTTON_KEYS = [
  'south',
  'east',
  'west',
  'north',
  'l1',
  'r1',
  'l2',
  'r2',
  'l3',
  'r3',
  'view',
  'menu',
  'home',
  'dpadUp',
  'dpadDown',
  'dpadLeft',
  'dpadRight',
] as const satisfies readonly (keyof LogicalButtonsStateDto)[]

export type ButtonKey = typeof BUTTON_KEYS[number]

export function neutralState(): LogicalPadStateDto {
  return {
    buttons: Object.fromEntries(BUTTON_KEYS.map(key => [key, 0])) as unknown as LogicalButtonsStateDto,
    leftStick: { x: 0, y: 0 },
    rightStick: { x: 0, y: 0 },
    leftTrigger: 0,
    rightTrigger: 0,
  }
}

export function clonePadState(state: LogicalPadStateDto): LogicalPadStateDto {
  return {
    buttons: { ...state.buttons },
    leftStick: { ...state.leftStick },
    rightStick: { ...state.rightStick },
    leftTrigger: state.leftTrigger,
    rightTrigger: state.rightTrigger,
  }
}

export function axisValue(value: number | undefined): number {
  return Number.isFinite(value) ? Math.max(-1, Math.min(1, value!)) : 0
}

export function buttonValue(value: number | undefined): number {
  return Math.max(0, axisValue(value))
}

export function stateEquals(a: LogicalPadStateDto, b: LogicalPadStateDto): boolean {
  return BUTTON_KEYS.every(key => a.buttons[key] === b.buttons[key])
    && a.leftStick.x === b.leftStick.x && a.leftStick.y === b.leftStick.y
    && a.rightStick.x === b.rightStick.x && a.rightStick.y === b.rightStick.y
    && a.leftTrigger === b.leftTrigger && a.rightTrigger === b.rightTrigger
}

/** 交接只屏蔽当时按住的控件，各自归位后独立恢复。 */
export class PadReleaseBarrier {
  private buttons = new Set<ButtonKey>()
  private sticks = new Set<'leftStick' | 'rightStick'>()
  private triggers = new Set<'leftTrigger' | 'rightTrigger'>()

  constructor(private readonly leftDeadzone = 0.1, private readonly rightDeadzone = leftDeadzone) {}

  private deadzone(key: 'leftStick' | 'rightStick'): number {
    // 归位容差保留最小值，允许用户将游戏死区设为零。
    return Math.max(0.03, key === 'leftStick' ? this.leftDeadzone : this.rightDeadzone)
  }

  capture(state: LogicalPadStateDto): void {
    this.buttons = new Set(BUTTON_KEYS.filter(key => state.buttons[key] > 0.5))
    this.sticks = new Set((['leftStick', 'rightStick'] as const).filter(key =>
      Math.hypot(state[key].x, state[key].y) > this.deadzone(key),
    ))
    this.triggers = new Set((['leftTrigger', 'rightTrigger'] as const).filter(key => state[key] > 0.03))
  }

  apply(state: LogicalPadStateDto): LogicalPadStateDto {
    const result = clonePadState(state)
    for (const key of this.buttons) {
      if (state.buttons[key] <= 0.5)
        this.buttons.delete(key)
      result.buttons[key] = 0
    }
    for (const key of this.sticks) {
      if (Math.hypot(state[key].x, state[key].y) <= this.deadzone(key))
        this.sticks.delete(key)
      result[key] = { x: 0, y: 0 }
    }
    for (const key of this.triggers) {
      if (state[key] <= 0.03)
        this.triggers.delete(key)
      result[key] = 0
      result.buttons[key === 'leftTrigger' ? 'l2' : 'r2'] = 0
    }
    return result
  }
}
