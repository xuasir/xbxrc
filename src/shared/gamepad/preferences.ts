import type { GamepadBackendKind } from './backend'
import type { LogicalPadStateDto } from './contract'
import type { ButtonKey } from './state'
import { BUTTON_KEYS, clonePadState } from './state'

export type BackgroundPolicy = 'follow-app' | 'keep-active'
export type InputOwner = 'ui' | 'stream' | 'blocked'
export type GameControlSource = 'gamepad' | 'keyboard'

export interface GameControls {
  buttons: Record<ButtonKey, ButtonKey>
  leftDeadzone: number
  rightDeadzone: number
  invertRightY: boolean
}

export interface GamepadPreferences {
  backend: GamepadBackendKind
  backgroundPolicy: BackgroundPolicy
  controlSource: GameControlSource
  gameControls: GameControls
  vibration: boolean
  vibrationStrength: 'realistic' | 'enhanced' | 'full'
  uiHaptics: boolean
}

export const GAMEPAD_CONFIG_KEYS = [
  'gamepad_backend',
  'gamepad_background_policy',
  'gamepad_control_source',
  'gamepad_game_controls',
  'gamepad_keyboard_mapping',
  'vibration',
  'vibration_strength',
  'ui_haptics',
] as const

export function defaultGameControls(): GameControls {
  return {
    buttons: Object.fromEntries(BUTTON_KEYS.map(key => [key, key])) as Record<ButtonKey, ButtonKey>,
    leftDeadzone: 0.1,
    rightDeadzone: 0.1,
    invertRightY: false,
  }
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {}
}

export function parseGameControls(value: unknown): GameControls {
  const raw = record(value)
  const result = defaultGameControls()
  const mapping = record(raw.buttons)
  const entries = BUTTON_KEYS.map(key => mapping[key] ?? key)
  // 接入布局固定为双射；扳机保留模拟值，系统键保持原义。
  if (entries.every(key => BUTTON_KEYS.includes(key as ButtonKey))
    && new Set(entries).size === BUTTON_KEYS.length
    && (mapping.home ?? 'home') === 'home'
    && ['l2', 'r2'].includes(String(mapping.l2 ?? 'l2'))
    && ['l2', 'r2'].includes(String(mapping.r2 ?? 'r2'))) {
    result.buttons = Object.fromEntries(BUTTON_KEYS.map((key, i) => [key, entries[i]])) as Record<ButtonKey, ButtonKey>
  }
  for (const key of ['leftDeadzone', 'rightDeadzone'] as const) {
    const value = raw[key]
    if (typeof value === 'number' && Number.isFinite(value))
      result[key] = Math.max(0, Math.min(0.3, value))
  }
  result.invertRightY = raw.invertRightY === true
  return result
}

export function parseGamepadPreferences(value: unknown): GamepadPreferences {
  const raw = record(value)
  return {
    backend: raw.gamepad_backend === 'native' ? 'native' : 'browser',
    // 平台来源未知时保持消费；有效用户选择始终优先。
    backgroundPolicy: raw.gamepad_background_policy === 'follow-app' ? 'follow-app' : 'keep-active',
    controlSource: raw.gamepad_control_source === 'keyboard' ? 'keyboard' : 'gamepad',
    gameControls: parseGameControls(raw.gamepad_game_controls),
    vibration: raw.vibration !== false,
    vibrationStrength: raw.vibration_strength === 'full' || raw.vibration_strength === 'enhanced'
      ? raw.vibration_strength
      : 'realistic',
    uiHaptics: raw.ui_haptics === true,
  }
}

export function allowsInput(policy: BackgroundPolicy, isBackground: boolean | undefined): boolean {
  return policy === 'keep-active' || isBackground !== true
}

export function deriveInputOwner(
  policy: BackgroundPolicy,
  isBackground: boolean | undefined,
  streamReady: boolean,
  uiCapture: boolean,
): InputOwner {
  if (!allowsInput(policy, isBackground))
    return 'blocked'
  return streamReady && !uiCapture ? 'stream' : 'ui'
}

function deadzone(stick: { x: number, y: number }, zone: number): { x: number, y: number } {
  const magnitude = Math.hypot(stick.x, stick.y)
  if (magnitude <= zone || magnitude === 0)
    return { x: 0, y: 0 }
  const scale = Math.min(1, (magnitude - zone) / (1 - zone)) / magnitude
  return { x: stick.x * scale, y: stick.y * scale }
}

/** UI消费标准态；此变换仅用于游戏分支。 */
export function applyGameControls(state: LogicalPadStateDto, config: GameControls): LogicalPadStateDto {
  const output = clonePadState(state)
  for (const key of BUTTON_KEYS)
    output.buttons[key] = state.buttons[config.buttons[key]]
  output.leftTrigger = config.buttons.l2 === 'r2' ? state.rightTrigger : state.leftTrigger
  output.rightTrigger = config.buttons.r2 === 'l2' ? state.leftTrigger : state.rightTrigger
  output.leftTrigger = output.leftTrigger <= 0.03 ? 0 : output.leftTrigger
  output.rightTrigger = output.rightTrigger <= 0.03 ? 0 : output.rightTrigger
  output.buttons.l2 = output.leftTrigger
  output.buttons.r2 = output.rightTrigger
  output.leftStick = deadzone(state.leftStick, config.leftDeadzone)
  output.rightStick = deadzone(state.rightStick, config.rightDeadzone)
  if (config.invertRightY)
    output.rightStick.y *= -1
  return output
}
