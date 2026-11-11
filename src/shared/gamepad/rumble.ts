import type { GamepadRumbleEffectDto } from './contract'
import type { GamepadPreferences } from './preferences'
import { buttonValue } from './state'

const PRESETS = {
  realistic: { handleScale: 0.66, handleGamma: 1.32, triggerScale: 0.58, triggerGamma: 1.26 },
  enhanced: { handleScale: 0.82, handleGamma: 1.18, triggerScale: 0.74, triggerGamma: 1.14 },
  full: { handleScale: 1, handleGamma: 1, triggerScale: 1, triggerGamma: 1 },
}

export function gameRumbleEffect(effect: GamepadRumbleEffectDto, strength: GamepadPreferences['vibrationStrength'], channels?: GamepadPreferences['vibrationChannels']): GamepadRumbleEffectDto {
  const motor = (value: number, trigger: boolean, channel: keyof NonNullable<typeof channels>) => {
    const preset = PRESETS[channels?.[channel] ?? strength]
    return buttonValue(value) ** (trigger ? preset.triggerGamma : preset.handleGamma) * (trigger ? preset.triggerScale : preset.handleScale)
  }
  return {
    ...effect,
    strongMagnitude: motor(effect.strongMagnitude, false, 'leftGrip'),
    weakMagnitude: motor(effect.weakMagnitude, false, 'rightGrip'),
    leftTrigger: motor(effect.leftTrigger, true, 'leftTrigger'),
    rightTrigger: motor(effect.rightTrigger, true, 'rightTrigger'),
  }
}
