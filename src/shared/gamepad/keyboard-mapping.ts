import type { GamepadKeyboardControlDto, GamepadKeyboardKeyDto, GamepadKeyboardMappingDto } from './contract'
import { BUTTON_KEYS } from './state'

export const KEYBOARD_DIRECTIONS: GamepadKeyboardControlDto[] = ['leftStickUp', 'leftStickDown', 'leftStickLeft', 'leftStickRight', 'rightStickUp', 'rightStickDown', 'rightStickLeft', 'rightStickRight']
export function validKeyboardKey(key: unknown): key is GamepadKeyboardKeyDto {
  return typeof key === 'string' && (/^key[A-Z]$/.test(key) || /^digit\d$/.test(key) || ['enter', 'tab', 'escape', 'space', 'arrowUp', 'arrowDown', 'arrowLeft', 'arrowRight'].includes(key))
}
export function parseKeyboardMapping(value: unknown, fallback: GamepadKeyboardMappingDto): GamepadKeyboardMappingDto {
  const raw = value as Partial<GamepadKeyboardMappingDto> | undefined
  const validControl = (control: GamepadKeyboardControlDto) => BUTTON_KEYS.includes(control as typeof BUTTON_KEYS[number]) || KEYBOARD_DIRECTIONS.includes(control)
  const seen = new Set<string>()
  const bindings = (Array.isArray(raw?.bindings) ? raw.bindings.slice(0, 128) : []).filter(binding => binding && validKeyboardKey(binding.key) && validControl(binding.control) && !seen.has(binding.key) && !!seen.add(binding.key)).map(({ key, control }) => ({ key, control }))
  const targets = new Set<string>()
  const chords = (Array.isArray(raw?.chords) ? raw.chords.slice(0, 25) : []).filter((binding) => {
    if (!binding || !validControl(binding.control) || targets.has(binding.control) || !Array.isArray(binding.keys) || binding.keys.length < 2 || binding.keys.length > 4 || !binding.keys.every(validKeyboardKey) || new Set(binding.keys).size !== binding.keys.length)
      return false
    const key = [...binding.keys].sort().join(',')
    if (seen.has(key))
      return false
    seen.add(key)
    targets.add(binding.control)
    return true
  }).map(({ keys, control }) => ({ keys: [...keys], control }))
  return bindings.length || chords.length ? { bindings, ...(chords.length ? { chords } : {}) } : structuredClone(fallback)
}
export function assignKeyboardInput(mapping: GamepadKeyboardMappingDto, control: GamepadKeyboardControlDto, keys: GamepadKeyboardKeyDto[]): boolean {
  if (!keys.length || keys.length > 4 || new Set(keys).size !== keys.length)
    return false
  const key = [...keys].sort().join(',')
  mapping.bindings = mapping.bindings.filter(binding => binding.control !== control && (keys.length > 1 || binding.key !== keys[0]))
  const chords = (mapping.chords ?? []).filter(binding => binding.control !== control && [...binding.keys].sort().join(',') !== key)
  if (keys.length === 1)
    mapping.bindings.push({ key: keys[0], control })
  else chords.push({ keys: [...keys], control })
  if (chords.length)
    mapping.chords = chords
  else delete mapping.chords
  return true
}
