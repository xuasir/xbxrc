import { describe, expect, it } from 'vitest'
import { parseGamepadPreferences } from './preferences'
import { gameRumbleEffect } from './rumble'

describe('four-channel rumble preferences', () => {
  it('inherits the old strength and tunes all four motors independently', () => {
    const old = parseGamepadPreferences({ vibration_strength: 'enhanced' })
    expect(Object.values(old.vibrationChannels)).toEqual(['enhanced', 'enhanced', 'enhanced', 'enhanced'])
    const prefs = parseGamepadPreferences({ vibration_channels: { leftGrip: 'full', rightGrip: 'realistic', leftTrigger: 'enhanced', rightTrigger: 'full' } })
    const result = gameRumbleEffect({ durationMs: 200, startDelayMs: 0, strongMagnitude: 1, weakMagnitude: 1, leftTrigger: 1, rightTrigger: 1, repeat: 0 }, prefs.vibrationStrength, prefs.vibrationChannels)
    expect(result).toMatchObject({ strongMagnitude: 1, weakMagnitude: 0.66, leftTrigger: 0.74, rightTrigger: 1 })
  })
})
