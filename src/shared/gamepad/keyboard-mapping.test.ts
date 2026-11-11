import { describe, expect, it } from 'vitest'
import { assignKeyboardInput, parseKeyboardMapping } from './keyboard-mapping'

const defaults = { bindings: [{ key: 'keyJ' as const, control: 'south' as const }] }
describe('keyboard target mapping', () => {
  it('binds simultaneous keys while retaining single-key alternatives for other targets', () => {
    const mapping = parseKeyboardMapping(defaults, defaults)
    expect(assignKeyboardInput(mapping, 'home', ['keyJ', 'keyK'])).toBe(true)
    expect(mapping).toMatchObject({ bindings: defaults.bindings, chords: [{ control: 'home', keys: ['keyJ', 'keyK'] }] })
    expect(parseKeyboardMapping(JSON.parse(JSON.stringify(mapping)), defaults)).toEqual(mapping)
    expect(assignKeyboardInput(mapping, 'south', ['keyK', 'keyJ'])).toBe(true)
    expect(mapping.chords).toEqual([{ control: 'south', keys: ['keyK', 'keyJ'] }])
    expect(assignKeyboardInput(mapping, 'south', ['keyL'])).toBe(true)
    expect(mapping.chords).toBeUndefined()
    expect(mapping.bindings).toEqual([{ key: 'keyL', control: 'south' }])
  })
  it('bounds and validates bindings, controls and duplicate combinations with default fallback', () => {
    expect(parseKeyboardMapping({ bindings: [{ key: 'invalid', control: 'south' }] }, defaults)).toEqual(defaults)
    expect(parseKeyboardMapping({ bindings: [], chords: [{ keys: ['keyJ', 'keyK'], control: 'home' }, { keys: ['keyK', 'keyJ'], control: 'east' }, { keys: ['keyJ', 'keyJ'], control: 'west' }] }, defaults)).toEqual({ bindings: [], chords: [{ keys: ['keyJ', 'keyK'], control: 'home' }] })
    const mapping = structuredClone(defaults)
    expect(assignKeyboardInput(mapping, 'home', ['keyA', 'keyB', 'keyC', 'keyD', 'keyE'])).toBe(false)
    expect(mapping).toEqual(defaults)
  })
})
