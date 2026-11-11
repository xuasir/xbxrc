import type { RawPadState } from './backend'
import type { GamepadKeyboardControlDto, LogicalPadStateDto } from './contract'
import type { DeviceInputBinding, MappingTarget } from './device-mapping'
import type { GameControls } from './preferences'
import type { ButtonKey } from './state'
import { defaultDeviceMapping, parseDeviceMappings, STANDARD_INPUT_BUTTONS, STICK_AXIS_TARGETS } from './device-mapping'
import { EXTRA_BUTTON_TARGETS } from './preferences'
import { MAX_EXTRA_BINDINGS, MAX_LAYOUT_KEY_LENGTH, MAX_RAW_BUTTONS, RawButtonReleaseBarrier, STANDARD_BUTTON_COUNT } from './raw-input'
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

export type GamepadCaptureAction = { kind: 'buttons', buttons: number[] } | { kind: 'axis', axis: number, positive: boolean, rest: number }

export function mappingTarget(control: GamepadKeyboardControlDto): MappingTarget {
  if (control.startsWith('leftStick') || control.startsWith('rightStick'))
    return `${control.startsWith('left') ? 'left' : 'right'}Stick${control.endsWith('Up') || control.endsWith('Down') ? 'Y' : 'X'}` as MappingTarget
  return control as ButtonKey
}

/** 入口控件先释放；记录最大同时按住集合，全部释放/回中后提交。 */
export function createGamepadCaptureReader(initial: LogicalPadStateDto, initialRaw?: RawPadState, target: GamepadKeyboardControlDto = 'south') {
  const barrier = new PadReleaseBarrier()
  const rawBarrier = new RawButtonReleaseBarrier()
  const axisOnly = STICK_AXIS_TARGETS.includes(mappingTarget(target) as typeof STICK_AXIS_TARGETS[number])
  const rawInitial = initialRaw ?? { buttons: STANDARD_INPUT_BUTTONS.map(key => initial.buttons[key]), axes: [initial.leftStick.x, initial.leftStick.y, initial.rightStick.x, initial.rightStick.y] }
  const rests = rawInitial.axes.map(value => !axisOnly && Math.abs(value) > 0.8 ? value : 0)
  barrier.capture(initial)
  rawBarrier.capture(rawInitial, new Map(rests.map((rest, axis) => [axis, rest])))
  let peak: number[] = []
  let axisPeak: Extract<GamepadCaptureAction, { kind: 'axis' }> | undefined
  let axisMagnitude = 0.45
  return (state: LogicalPadStateDto, raw?: RawPadState): GamepadCaptureAction | undefined => {
    const next = barrier.apply(state)
    const sample = rawBarrier.apply(raw ?? { buttons: STANDARD_INPUT_BUTTONS.map(key => next.buttons[key]), axes: [next.leftStick.x, next.leftStick.y, next.rightStick.x, next.rightStick.y] })!
    const held = sample.buttons.flatMap((value, index) => value > 0.5 ? [index] : [])
    if (!axisOnly && held.length > peak.length)
      peak = held
    sample.axes.forEach((value, axis) => {
      const delta = value - (rests[axis] ?? 0)
      if (Math.abs(delta) > axisMagnitude) {
        axisMagnitude = Math.abs(delta)
        axisPeak = { kind: 'axis', axis, positive: delta > 0, rest: rests[axis] ?? 0 }
      }
    })
    if (held.length)
      return undefined
    if (peak.length) {
      const buttons = peak
      peak = []
      axisPeak = undefined
      axisMagnitude = 0.45
      return { kind: 'buttons', buttons }
    }
    if (axisPeak && sample.axes.every((value, axis) => Math.abs(value - (rests[axis] ?? 0)) <= 0.15)) {
      const action = axisPeak
      axisPeak = undefined
      axisMagnitude = 0.45
      return action
    }
    return undefined
  }
}

export function assignCapturedGameButton(controls: GameControls, layout: string, control: GamepadKeyboardControlDto, action: GamepadCaptureAction, standard: boolean): boolean {
  const target = mappingTarget(control)
  const existing = controls.deviceMappings.find(item => item.layout === layout)
  const mapping = JSON.parse(JSON.stringify(existing ?? defaultDeviceMapping(layout, standard, controls.buttons, controls.extraButtons))) as ReturnType<typeof defaultDeviceMapping>
  const previous = mapping.bindings.find(binding => binding.target === target)
  let binding: DeviceInputBinding
  if (action.kind === 'buttons') {
    if (!BUTTON_KEYS.includes(target as ButtonKey))
      return false
    binding = { target: target as ButtonKey, kind: 'buttons', buttons: action.buttons, mode: 'all' }
    if (action.buttons.length === 1 && previous?.kind === 'buttons' && previous.buttons.length === 1) {
      const other = mapping.bindings.find(item => item.target !== target && item.kind === 'buttons' && item.buttons.length === 1 && item.buttons[0] === action.buttons[0])
      if (other?.kind === 'buttons')
        other.buttons = [...previous.buttons]
    }
  }
  else {
    const stick = STICK_AXIS_TARGETS.includes(target as typeof STICK_AXIS_TARGETS[number])
    const positive = control.endsWith('Right') || control.endsWith('Down') || !stick
    binding = { target, kind: 'axis', axis: action.axis, invert: positive !== action.positive, range: stick ? 'signed' : Math.abs(action.rest) > 0.8 ? 'full' : 'positive' }
    if (stick && previous?.kind === 'axis') {
      const other = mapping.bindings.find(item => item.target !== target && item.kind === 'axis' && item.range === 'signed' && item.axis === action.axis)
      if (other?.kind === 'axis') {
        other.axis = previous.axis
        other.invert = previous.invert
      }
    }
  }
  mapping.bindings = [...mapping.bindings.filter(item => item.target !== target), binding]
  const parsed = parseDeviceMappings([mapping])[0]
  if (!parsed || parsed.bindings.length !== mapping.bindings.length)
    return false
  const profiles = [...controls.deviceMappings.filter(item => item.layout !== layout), parsed]
  if (profiles.length > 64)
    return false
  controls.deviceMappings = profiles
  return true
}

/** 进入时持键先释放；长按起点用单调时钟，任意一键释放立即清零。 */
export function createGamepadExitReader(initial: LogicalPadStateDto) {
  const barrier = new PadReleaseBarrier()
  barrier.capture(initial)
  let startedAt: number | null = null
  return (state: LogicalPadStateDto, now: number): number => {
    const next = barrier.apply(state)
    if (next.buttons.menu <= 0.5 || next.buttons.view <= 0.5) {
      startedAt = null
      return 0
    }
    startedAt ??= now
    return Math.max(0, Math.min(1, (now - startedAt) / 1000))
  }
}

export function assignExtraButton(controls: GameControls, layout: string, button: number, target: ButtonKey): boolean {
  if (!EXTRA_BUTTON_TARGETS.includes(target) || !layout.length || layout.length > MAX_LAYOUT_KEY_LENGTH
    || !Number.isInteger(button) || button < STANDARD_BUTTON_COUNT || button >= MAX_RAW_BUTTONS) {
    return false
  }
  const existing = controls.extraButtons.find(binding => binding.layout === layout && binding.button === button)
  if (existing)
    existing.target = target
  else if (controls.extraButtons.length < MAX_EXTRA_BINDINGS)
    controls.extraButtons.push({ layout, button, target })
  else return false
  return true
}

export function assignGameButton(controls: GameControls, target: ButtonKey, source: ButtonKey): boolean {
  const triggers: ButtonKey[] = ['l2', 'r2']
  if (triggers.includes(target) !== triggers.includes(source))
    return false
  const other = BUTTON_KEYS.find(key => controls.buttons[key] === source)!
  controls.buttons[other] = controls.buttons[target]
  controls.buttons[target] = source
  return true
}
