import type { RenderControlsValue } from '../shared/config/render-controls'
import type { StreamConfigSnapshot } from './types'
import { computed, shallowRef } from 'vue'
import { copyRenderControls, normalizeRenderControls } from '../shared/config/render-controls'

/** 本次页面的显示意图；启动快照、已应用覆盖与临时预览分开持有。 */
export function useSessionRenderControls() {
  const defaults = shallowRef(normalizeRenderControls({}))
  const override = shallowRef<RenderControlsValue | null>(null)
  const preview = shallowRef<RenderControlsValue | null>(null)
  const temporary = shallowRef<RenderControlsValue | null>(null)
  const current = computed(() => override.value ?? defaults.value)
  const active = computed(() => preview.value ?? temporary.value ?? current.value)
  const source = computed(() => override.value === null ? 'default' : 'session')
  let initialized = false
  let revision = 0

  function clearTransient(): void {
    revision++
    preview.value = null
    temporary.value = null
  }

  function apply(value: RenderControlsValue): void {
    clearTransient()
    override.value = copyRenderControls(value)
  }

  return {
    defaults,
    current,
    active,
    source,
    getRevision: () => revision,
    initialize(config: StreamConfigSnapshot) {
      if (!initialized) {
        defaults.value = normalizeRenderControls(config)
        initialized = true
      }
    },
    apply,
    preview(value: RenderControlsValue) {
      clearTransient()
      preview.value = copyRenderControls(value)
    },
    clearTransient,
    restoreDefaults() {
      clearTransient()
      override.value = null
    },
    acceptSavedDefault(value: RenderControlsValue) {
      clearTransient()
      defaults.value = copyRenderControls(value)
      override.value = null
    },
    beginTemporary(value: RenderControlsValue): () => void {
      clearTransient()
      const token = revision
      temporary.value = copyRenderControls(value)
      return () => {
        if (token === revision)
          clearTransient()
      }
    },
  }
}
