import type {
  GamepadKeyboardControlDto,
  GamepadKeyboardKeyDto,
  GamepadKeyboardMappingDto,
  LogicalPadStateDto,
} from './contract'
import { rpc } from '../../services/rpc'

const DEFAULT_KEYBOARD_MAPPING: GamepadKeyboardMappingDto = {
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

const KEY_CODE_TO_DTO: Record<string, GamepadKeyboardKeyDto | undefined> = {
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

let installed = false
let mapping = DEFAULT_KEYBOARD_MAPPING
let submitSeq = 0
const pressedKeys = new Set<GamepadKeyboardKeyDto>()

export function installKeyboardFallbackBridge(): void {
  if (installed) {
    return
  }
  installed = true
  void loadMapping()

  window.addEventListener('keydown', handleKeyDown, true)
  window.addEventListener('keyup', handleKeyUp, true)
  window.addEventListener('blur', clearPressedKeys)
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState !== 'visible') {
      clearPressedKeys()
    }
  })
}

async function loadMapping(): Promise<void> {
  try {
    const config = await rpc.config.get({ keys: ['gamepad_keyboard_mapping'] }) as {
      gamepad_keyboard_mapping?: GamepadKeyboardMappingDto
    }
    if (config.gamepad_keyboard_mapping?.bindings?.length) {
      mapping = config.gamepad_keyboard_mapping
    }
  }
  catch {
    mapping = DEFAULT_KEYBOARD_MAPPING
  }
}

function handleKeyDown(event: KeyboardEvent): void {
  const key = KEY_CODE_TO_DTO[event.code]
  if (key === undefined || isEditableTarget(event.target)) {
    return
  }
  event.preventDefault()
  if (!pressedKeys.has(key)) {
    pressedKeys.add(key)
    submitCurrentState('keydown')
  }
}

function handleKeyUp(event: KeyboardEvent): void {
  const key = KEY_CODE_TO_DTO[event.code]
  if (key === undefined || isEditableTarget(event.target)) {
    return
  }
  event.preventDefault()
  if (pressedKeys.delete(key)) {
    submitCurrentState('keyup')
  }
}

function clearPressedKeys(): void {
  if (pressedKeys.size === 0) {
    return
  }
  pressedKeys.clear()
  submitCurrentState('clear')
}

function submitCurrentState(reason: string): void {
  const seq = ++submitSeq
  const state = buildState()
  void rpc.gamepad.submitKeyboardState({ state })
    .then(() => {
      void rpc.runtimeTrace.recordEvent({
        event: 'keyboardFallbackDomStateSubmitted',
        payload: {
          activeControlCount: countActiveControls(state),
          pressedKeyCount: pressedKeys.size,
          reason,
          seq,
        },
      }).catch(() => {})
    })
    .catch(() => {})
}

function buildState(): LogicalPadStateDto {
  const state = defaultLogicalPadState()
  for (const binding of mapping.bindings.length ? mapping.bindings : DEFAULT_KEYBOARD_MAPPING.bindings) {
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

function defaultLogicalPadState(): LogicalPadStateDto {
  return {
    buttons: {
      south: 0,
      east: 0,
      west: 0,
      north: 0,
      l1: 0,
      r1: 0,
      l2: 0,
      r2: 0,
      l3: 0,
      r3: 0,
      view: 0,
      menu: 0,
      home: 0,
      dpadUp: 0,
      dpadDown: 0,
      dpadLeft: 0,
      dpadRight: 0,
    },
    leftStick: { x: 0, y: 0 },
    rightStick: { x: 0, y: 0 },
    leftTrigger: 0,
    rightTrigger: 0,
  }
}

function countActiveControls(state: LogicalPadStateDto): number {
  return [
    ...Object.values(state.buttons),
    state.leftStick.x,
    state.leftStick.y,
    state.rightStick.x,
    state.rightStick.y,
    state.leftTrigger,
    state.rightTrigger,
  ].filter(value => Math.abs(value) > 0.001).length
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
