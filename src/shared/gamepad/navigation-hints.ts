import type { GamepadKeyboardMappingDto } from './contract'
import type { GameControlSource } from './preferences'
import type { ButtonKey } from './state'
import { computed, onScopeDispose, shallowRef } from 'vue'
import defaultKeyboardMapping from './keyboard-defaults.json'
import { inputController } from './runtime'
import { BUTTON_LABELS } from './settings-model'
import { BUTTON_KEYS } from './state'

const menuMapping = defaultKeyboardMapping as GamepadKeyboardMappingDto

/** 菜单使用固定键位；Enter / Tab 保留原生表单行为，提示采用 8 / 7。 */
export function navigationButtonLabels(source: GameControlSource): Record<ButtonKey, string> {
  if (source === 'gamepad')
    return BUTTON_LABELS
  return Object.fromEntries(BUTTON_KEYS.map(control => [control, menuMapping.bindings
    .filter(binding => binding.control === control && binding.key !== 'enter' && binding.key !== 'tab')
    .map(binding => binding.key.replace(/^(key|digit)/, '')
      .replace('arrowUp', '↑')
      .replace('arrowDown', '↓')
      .replace('arrowLeft', '←')
      .replace('arrowRight', '→'))
    .join(' / ')])) as Record<ButtonKey, string>
}

export function useNavigationButtonLabels() {
  const source = shallowRef(inputController.getSnapshot().preferences.controlSource)
  const dispose = inputController.subscribe(snapshot => source.value = snapshot.preferences.controlSource)
  onScopeDispose(dispose)
  return computed(() => navigationButtonLabels(source.value))
}
