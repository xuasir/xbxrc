import type { GamepadBackendKind, RawPadState } from './backend'
import type { LogicalPadStateDto } from './contract'
import type { DeviceMapping } from './device-mapping'
import type { ButtonKey } from './state'
import { applyDeviceMapping, defaultDeviceMapping, parseDeviceMappings } from './device-mapping'
import { MAX_EXTRA_BINDINGS, MAX_LAYOUT_KEY_LENGTH, MAX_RAW_BUTTONS, STANDARD_BUTTON_COUNT } from './raw-input'
import { BUTTON_KEYS, clonePadState } from './state'

export const EXTRA_BUTTON_TARGETS = BUTTON_KEYS.filter(key => !['l2', 'r2'].includes(key))
export interface ExtraButtonBinding {
  layout: string
  button: number
  target: ButtonKey
}

export type BackgroundPolicy = 'follow-app' | 'keep-active'
export type InputOwner = 'ui' | 'stream' | 'blocked'
export type GameControlSource = 'gamepad' | 'keyboard'
export const RUMBLE_CHANNELS = ['leftGrip', 'rightGrip', 'leftTrigger', 'rightTrigger'] as const
export type RumbleChannel = typeof RUMBLE_CHANNELS[number]
export type VibrationStrength = 'realistic' | 'enhanced' | 'full'
export type StreamShortcut = 'menu' | 'diagnostics'

export interface GameControls {
  buttons: Record<ButtonKey, ButtonKey>
  leftDeadzone: number
  rightDeadzone: number
  invertLeftY: boolean
  invertRightY: boolean
  extraButtons: ExtraButtonBinding[]
  deviceMappings: DeviceMapping[]
}

export interface GamepadPreferences {
  backend: GamepadBackendKind
  backgroundPolicy: BackgroundPolicy
  controlSource: GameControlSource
  gameControls: GameControls
  vibration: boolean
  vibrationStrength: VibrationStrength
  vibrationChannels: Record<RumbleChannel, VibrationStrength>
  streamShortcuts: Record<StreamShortcut, ButtonKey[]>
  uiHaptics: boolean
}

export const GAMEPAD_CONFIG_KEYS = [
  'gamepad_background_policy',
  'gamepad_control_source',
  'gamepad_game_controls',
  'gamepad_keyboard_mapping',
  'vibration',
  'vibration_strength',
  'vibration_channels',
  'stream_menu_shortcuts',
  'ui_haptics',
] as const

export function defaultGameControls(): GameControls {
  return {
    buttons: Object.fromEntries(BUTTON_KEYS.map(key => [key, key])) as Record<ButtonKey, ButtonKey>,
    leftDeadzone: 0.1,
    rightDeadzone: 0.1,
    invertLeftY: false,
    invertRightY: false,
    extraButtons: [],
    deviceMappings: [],
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
  // 标准按钮保持双射；扳机保留模拟角色，Home 参与数字按钮映射。
  if (entries.every(key => BUTTON_KEYS.includes(key as ButtonKey))
    && new Set(entries).size === BUTTON_KEYS.length
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
  result.invertLeftY = raw.invertLeftY === true
  const seen = new Set<string>()
  for (const candidate of Array.isArray(raw.extraButtons) ? raw.extraButtons.slice(0, MAX_EXTRA_BINDINGS) : []) {
    const binding = record(candidate)
    if (typeof binding.layout !== 'string' || !binding.layout.length || binding.layout.length > MAX_LAYOUT_KEY_LENGTH
      || typeof binding.button !== 'number' || !Number.isInteger(binding.button) || binding.button < STANDARD_BUTTON_COUNT || binding.button >= MAX_RAW_BUTTONS
      || !EXTRA_BUTTON_TARGETS.includes(binding.target as ButtonKey)) {
      continue
    }
    const key = JSON.stringify([binding.layout, binding.button])
    if (seen.has(key))
      continue
    seen.add(key)
    result.extraButtons.push({ layout: binding.layout, button: binding.button, target: binding.target as ButtonKey })
  }
  result.deviceMappings = parseDeviceMappings(raw.deviceMappings)
  return result
}

export function parseGamepadPreferences(value: unknown): GamepadPreferences {
  const raw = record(value)
  const strength: VibrationStrength = raw.vibration_strength === 'full' || raw.vibration_strength === 'enhanced' ? raw.vibration_strength : 'realistic'
  const channels = record(raw.vibration_channels)
  const shortcuts = record(raw.stream_menu_shortcuts)
  const shortcut = (key: StreamShortcut, fallback: ButtonKey[]): ButtonKey[] => {
    const buttons = shortcuts[key]
    return Array.isArray(buttons) && buttons.length <= 4 && buttons.every(button => BUTTON_KEYS.includes(button)) && new Set(buttons).size === buttons.length ? [...buttons] : fallback
  }
  return {
    backend: 'browser',
    // 平台来源未知时保持消费；有效用户选择始终优先。
    backgroundPolicy: raw.gamepad_background_policy === 'follow-app' ? 'follow-app' : 'keep-active',
    controlSource: raw.gamepad_control_source === 'keyboard' ? 'keyboard' : 'gamepad',
    gameControls: parseGameControls(raw.gamepad_game_controls),
    vibration: raw.vibration !== false,
    vibrationStrength: strength,
    vibrationChannels: Object.fromEntries(RUMBLE_CHANNELS.map(key => [key, ['realistic', 'enhanced', 'full'].includes(String(channels[key])) ? channels[key] : strength])) as Record<RumbleChannel, VibrationStrength>,
    streamShortcuts: { menu: shortcut('menu', ['menu', 'view']), diagnostics: shortcut('diagnostics', []) },
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
export function applyGameControls(state: LogicalPadStateDto, config: GameControls, raw?: RawPadState, layoutKey?: string, standard = true): LogicalPadStateDto {
  let output = clonePadState(state)
  for (const key of BUTTON_KEYS)
    output.buttons[key] = state.buttons[config.buttons[key]]
  if (standard && raw && layoutKey) {
    for (const binding of config.extraButtons) {
      if (binding.layout === layoutKey)
        output.buttons[binding.target] = Math.max(output.buttons[binding.target], raw.buttons[binding.button] ?? 0)
    }
  }
  output.leftTrigger = config.buttons.l2 === 'r2' ? state.rightTrigger : state.leftTrigger
  output.rightTrigger = config.buttons.r2 === 'l2' ? state.leftTrigger : state.rightTrigger
  output.leftTrigger = output.leftTrigger <= 0.03 ? 0 : output.leftTrigger
  output.rightTrigger = output.rightTrigger <= 0.03 ? 0 : output.rightTrigger
  output.buttons.l2 = output.leftTrigger
  output.buttons.r2 = output.rightTrigger
  const mapping = config.deviceMappings.find(item => item.layout === layoutKey) ?? (!standard ? defaultDeviceMapping(layoutKey ?? '', false, config.buttons) : undefined)
  if (mapping)
    output = applyDeviceMapping(output, raw, mapping, standard)
  output.leftStick = deadzone(output.leftStick, config.leftDeadzone)
  output.rightStick = deadzone(output.rightStick, config.rightDeadzone)
  if (config.invertLeftY)
    output.leftStick.y *= -1
  if (config.invertRightY)
    output.rightStick.y *= -1
  return output
}
