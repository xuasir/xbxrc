import type {
  GamepadKeyboardControlDto,
  GamepadKeyboardKeyDto,
  GamepadKeyboardMappingDto,
  LogicalPadStateDto,
} from './contract'
import { inputController, subscribeGamepadConfig } from './runtime'
import { neutralState } from './state'

export const DEFAULT_KEYBOARD_MAPPING: GamepadKeyboardMappingDto = {
  bindings: [
    { key: 'keyW', control: 'leftStickUp' },
    { key: 'keyS', control: 'leftStickDown' },
    { key: 'keyA', control: 'leftStickLeft' },
    { key: 'keyD', control: 'leftStickRight' },
    { key: 'keyT', control: 'rightStickUp' },
    { key: 'keyG', control: 'rightStickDown' },
    { key: 'keyF', control: 'rightStickLeft' },
    { key: 'keyH', control: 'rightStickRight' },
    { key: 'keyJ', control: 'south' },
    { key: 'keyK', control: 'east' },
    { key: 'keyU', control: 'west' },
    { key: 'keyI', control: 'north' },
    { key: 'digit1', control: 'l1' },
    { key: 'digit2', control: 'r1' },
    { key: 'digit3', control: 'l2' },
    { key: 'digit4', control: 'r2' },
    { key: 'keyZ', control: 'l3' },
    { key: 'keyX', control: 'r3' },
    { key: 'tab', control: 'view' },
    { key: 'digit7', control: 'view' },
    { key: 'enter', control: 'menu' },
    { key: 'digit8', control: 'menu' },
    { key: 'digit9', control: 'home' },
    { key: 'arrowUp', control: 'dpadUp' },
    { key: 'arrowDown', control: 'dpadDown' },
    { key: 'arrowLeft', control: 'dpadLeft' },
    { key: 'arrowRight', control: 'dpadRight' },
  ],
}

export const KEY_CODE_TO_DTO: Record<string, GamepadKeyboardKeyDto | undefined> = {
  KeyA: 'keyA',
  KeyB: 'keyB',
  KeyC: 'keyC',
  KeyD: 'keyD',
  KeyE: 'keyE',
  KeyF: 'keyF',
  KeyG: 'keyG',
  KeyH: 'keyH',
  KeyI: 'keyI',
  KeyJ: 'keyJ',
  KeyK: 'keyK',
  KeyL: 'keyL',
  KeyM: 'keyM',
  KeyN: 'keyN',
  KeyO: 'keyO',
  KeyP: 'keyP',
  KeyQ: 'keyQ',
  KeyR: 'keyR',
  KeyS: 'keyS',
  KeyT: 'keyT',
  KeyU: 'keyU',
  KeyV: 'keyV',
  KeyW: 'keyW',
  KeyX: 'keyX',
  KeyY: 'keyY',
  KeyZ: 'keyZ',
  Digit0: 'digit0',
  Digit1: 'digit1',
  Digit2: 'digit2',
  Digit3: 'digit3',
  Digit4: 'digit4',
  Digit5: 'digit5',
  Digit6: 'digit6',
  Digit7: 'digit7',
  Digit8: 'digit8',
  Digit9: 'digit9',
  Enter: 'enter',
  Tab: 'tab',
  Escape: 'escape',
  Space: 'space',
  ArrowUp: 'arrowUp',
  ArrowDown: 'arrowDown',
  ArrowLeft: 'arrowLeft',
  ArrowRight: 'arrowRight',
}

let uninstall: (() => void) | undefined
let mapping = DEFAULT_KEYBOARD_MAPPING
let controlSource: unknown
const pressedKeys = new Set<GamepadKeyboardKeyDto>()

export function installKeyboardFallbackBridge(): () => void {
  if (uninstall)
    return uninstall
  const disposeConfig = subscribeGamepadConfig((config) => {
    const stored = config.gamepad_keyboard_mapping as GamepadKeyboardMappingDto | undefined
    const next = stored?.bindings?.length ? stored : DEFAULT_KEYBOARD_MAPPING
    if (JSON.stringify(mapping) !== JSON.stringify(next) || controlSource !== config.gamepad_control_source)
      clearPressedKeys()
    mapping = structuredClone(next)
    controlSource = config.gamepad_control_source
  })
  window.addEventListener('keydown', handleKeyDown, { capture: true })
  window.addEventListener('keyup', handleKeyUp, { capture: true })
  window.addEventListener('blur', clearPressedKeys)
  document.addEventListener('visibilitychange', handleVisibilityChange)
  const cleanup = () => {
    if (uninstall !== cleanup)
      return
    disposeConfig()
    window.removeEventListener('keydown', handleKeyDown, { capture: true })
    window.removeEventListener('keyup', handleKeyUp, { capture: true })
    window.removeEventListener('blur', clearPressedKeys)
    document.removeEventListener('visibilitychange', handleVisibilityChange)
    clearPressedKeys()
    uninstall = undefined
  }
  uninstall = cleanup
  return cleanup
}

function handleVisibilityChange(): void {
  if (document.visibilityState !== 'visible')
    clearPressedKeys()
}

if (import.meta.hot)
  import.meta.hot.dispose(() => uninstall?.())

function handleKeyDown(event: KeyboardEvent): void {
  const snapshot = inputController.getSnapshot()
  if (snapshot.preferences.controlSource !== 'keyboard' || event.ctrlKey || event.metaKey || event.altKey)
    return
  const key = KEY_CODE_TO_DTO[event.code]
  if (key === undefined || isEditableTarget(event.target) || !mapping.bindings.some(binding => binding.key === key)) {
    return
  }
  if (snapshot.owner === 'stream')
    event.preventDefault()
  // 保持浏览器默认动作抑制；持键重复等待真实松开与下一次按下。
  if (event.repeat)
    return
  if (!pressedKeys.has(key)) {
    pressedKeys.add(key)
    submitCurrentState()
  }
}

function handleKeyUp(event: KeyboardEvent): void {
  const key = KEY_CODE_TO_DTO[event.code]
  if (key && pressedKeys.delete(key)) {
    event.preventDefault()
    submitCurrentState()
  }
}

function clearPressedKeys(): void {
  if (pressedKeys.size === 0) {
    return
  }
  pressedKeys.clear()
  submitCurrentState()
}

function submitCurrentState(): void {
  inputController.setKeyboardState(buildState())
}

function buildState(): LogicalPadStateDto {
  const state = neutralState()
  for (const binding of mapping.bindings) {
    if (!pressedKeys.has(binding.key)) {
      continue
    }
    applyControl(state, binding.control)
  }
  state.leftStick.x = clampAxis(state.leftStick.x)
  state.leftStick.y = clampAxis(state.leftStick.y)
  state.rightStick.x = clampAxis(state.rightStick.x)
  state.rightStick.y = clampAxis(state.rightStick.y)
  return state
}

function applyControl(state: LogicalPadStateDto, control: GamepadKeyboardControlDto): void {
  switch (control) {
    case 'leftStickUp':
      state.leftStick.y -= 1
      break
    case 'leftStickDown':
      state.leftStick.y += 1
      break
    case 'leftStickLeft':
      state.leftStick.x -= 1
      break
    case 'leftStickRight':
      state.leftStick.x += 1
      break
    case 'rightStickUp':
      state.rightStick.y -= 1
      break
    case 'rightStickDown':
      state.rightStick.y += 1
      break
    case 'rightStickLeft':
      state.rightStick.x -= 1
      break
    case 'rightStickRight':
      state.rightStick.x += 1
      break
    case 'l2':
      state.leftTrigger = 1
      state.buttons.l2 = 1
      break
    case 'r2':
      state.rightTrigger = 1
      state.buttons.r2 = 1
      break
    case 'dpadUp':
      state.buttons.dpadUp = 1
      break
    case 'dpadDown':
      state.buttons.dpadDown = 1
      break
    case 'dpadLeft':
      state.buttons.dpadLeft = 1
      break
    case 'dpadRight':
      state.buttons.dpadRight = 1
      break
    default:
      state.buttons[control] = 1
      break
  }
}

function clampAxis(value: number): number {
  return Math.max(-1, Math.min(1, value))
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false
  }
  return target.isContentEditable
    || target instanceof HTMLInputElement
    || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement
}
