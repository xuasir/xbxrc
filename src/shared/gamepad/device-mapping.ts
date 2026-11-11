import type { RawPadState } from './backend'
import type { LogicalPadStateDto } from './contract'
import type { ButtonKey } from './state'
import { MAX_LAYOUT_KEY_LENGTH, MAX_RAW_AXES, MAX_RAW_BUTTONS } from './raw-input'
import { BUTTON_KEYS, neutralState } from './state'

export const STANDARD_INPUT_BUTTONS: ButtonKey[] = ['south', 'east', 'west', 'north', 'l1', 'r1', 'l2', 'r2', 'view', 'menu', 'l3', 'r3', 'dpadUp', 'dpadDown', 'dpadLeft', 'dpadRight', 'home']
export const STICK_AXIS_TARGETS = ['leftStickX', 'leftStickY', 'rightStickX', 'rightStickY'] as const
export type MappingTarget = ButtonKey | typeof STICK_AXIS_TARGETS[number]
export type DeviceInputBinding = { target: ButtonKey, kind: 'buttons', buttons: number[], mode: 'all' | 'any' }
  | { target: MappingTarget, kind: 'axis', axis: number, invert: boolean, range: 'signed' | 'positive' | 'full' }
export interface DeviceMapping { layout: string, bindings: DeviceInputBinding[] }
export const MAX_DEVICE_MAPPINGS = 64

export function parseDeviceMappings(value: unknown): DeviceMapping[] {
  const layouts = new Set<string>()
  return (Array.isArray(value) ? value.slice(0, MAX_DEVICE_MAPPINGS) : []).flatMap((item) => {
    if (!item || typeof item.layout !== 'string' || !item.layout.length || item.layout.length > MAX_LAYOUT_KEY_LENGTH || layouts.has(item.layout) || !Array.isArray(item.bindings))
      return []
    const targets = new Set<string>()
    const chords = new Set<string>()
    const bindings: DeviceInputBinding[] = []
    for (const binding of item.bindings.slice(0, 21)) {
      if (!binding || (!BUTTON_KEYS.includes(binding.target) && !STICK_AXIS_TARGETS.includes(binding.target)) || targets.has(binding.target))
        continue
      if (binding.kind === 'buttons') {
        const buttons = binding.buttons
        const mode = binding.mode === 'any' ? 'any' : 'all'
        if (!BUTTON_KEYS.includes(binding.target) || !Array.isArray(buttons) || buttons.length < 1 || buttons.length > (mode === 'any' ? MAX_RAW_BUTTONS : 4)
          || buttons.some(index => !Number.isInteger(index) || index < 0 || index >= MAX_RAW_BUTTONS) || new Set(buttons).size !== buttons.length) {
          continue
        }
        const key = [...buttons].sort((a, b) => a - b).join(',')
        if (mode === 'all' && buttons.length > 1 && chords.has(key))
          continue
        if (mode === 'all' && buttons.length > 1)
          chords.add(key)
        bindings.push({ target: binding.target, kind: 'buttons', buttons: [...buttons], mode })
      }
      else if (binding.kind === 'axis' && Number.isInteger(binding.axis) && binding.axis >= 0 && binding.axis < MAX_RAW_AXES) {
        const range = STICK_AXIS_TARGETS.includes(binding.target) ? 'signed' : binding.range === 'full' ? 'full' : 'positive'
        bindings.push({ target: binding.target, kind: 'axis', axis: binding.axis, invert: binding.invert === true, range })
      }
      else {
        continue
      }
      targets.add(binding.target)
    }
    layouts.add(item.layout)
    return [{ layout: item.layout, bindings }]
  })
}

export function defaultDeviceMapping(layout: string, standard: boolean, buttons: Record<ButtonKey, ButtonKey>, extras: { layout: string, target: ButtonKey, button: number }[] = []): DeviceMapping {
  return { layout, bindings: [
    ...(standard
      ? BUTTON_KEYS.map((target): DeviceInputBinding => {
          const sources = [STANDARD_INPUT_BUTTONS.indexOf(buttons[target]), ...extras.filter(item => item.layout === layout && item.target === target).map(item => item.button)]
          return { target, kind: 'buttons', buttons: [...new Set(sources)], mode: 'any' }
        })
      : []),
    ...STICK_AXIS_TARGETS.map((target, axis): DeviceInputBinding => ({ target, kind: 'axis', axis, invert: false, range: 'signed' })),
  ] }
}

function write(output: LogicalPadStateDto, target: MappingTarget, value: number): void {
  if (target === 'leftStickX')
    output.leftStick.x = value
  else if (target === 'leftStickY')
    output.leftStick.y = value
  else if (target === 'rightStickX')
    output.rightStick.x = value
  else if (target === 'rightStickY')
    output.rightStick.y = value
  else output.buttons[target] = value
}

/** 所有来源统一生成标准 Xbox 输出，组合成立时吸收组成按键的单键输出。 */
export function applyDeviceMapping(state: LogicalPadStateDto, raw: RawPadState | undefined, mapping: DeviceMapping, standard: boolean): LogicalPadStateDto {
  const output = neutralState()
  raw ??= standard ? { buttons: STANDARD_INPUT_BUTTONS.map(key => state.buttons[key]), axes: [state.leftStick.x, state.leftStick.y, state.rightStick.x, state.rightStick.y] } : undefined
  if (!raw)
    return output
  const activeChords = mapping.bindings.filter((binding): binding is Extract<DeviceInputBinding, { kind: 'buttons' }> => binding.kind === 'buttons' && binding.mode === 'all' && binding.buttons.length > 1 && binding.buttons.every(index => (raw.buttons[index] ?? 0) > 0.5))
  const consumed = new Set(activeChords.flatMap(binding => binding.buttons))
  const activeTargets = new Set(activeChords.map(binding => binding.target))
  for (const binding of mapping.bindings) {
    let value: number
    if (binding.kind === 'buttons') {
      value = binding.mode === 'all' && binding.buttons.length > 1 ? activeTargets.has(binding.target) ? 1 : 0 : Math.max(0, ...binding.buttons.map(index => consumed.has(index) ? 0 : raw.buttons[index] ?? 0))
    }
    else {
      const rest = binding.range === 'full' ? binding.invert ? 1 : -1 : 0
      const axis = Math.max(-1, Math.min(1, raw.axes[binding.axis] ?? rest)) * (binding.invert ? -1 : 1)
      value = binding.range === 'signed' ? axis : binding.range === 'full' ? (axis + 1) / 2 : Math.max(0, axis)
    }
    write(output, binding.target, value)
  }
  output.leftTrigger = output.buttons.l2
  output.rightTrigger = output.buttons.r2
  return output
}

export function deviceAxisRest(mapping?: DeviceMapping): Map<number, number> {
  const rests = new Map<number, number>()
  for (const binding of mapping?.bindings ?? []) {
    if (binding.kind === 'axis')
      rests.set(binding.axis, binding.range === 'full' ? binding.invert ? 1 : -1 : 0)
  }
  return rests
}
