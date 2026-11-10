import type { InjectionKey, Ref } from 'vue'
import { computed, inject, onBeforeUnmount, onDeactivated, onMounted, provide, ref, watch, watchEffect } from 'vue'
import { useRoute } from 'vue-router'
import { inputDispatcher } from '@/navigation/core/input'
import { inputController } from '@/shared/gamepad/runtime'

export interface UiMotionContext {
  enabled: Readonly<Ref<boolean>>
  setMediaActive: (owner: symbol, active: boolean) => void
}
const motionKey: InjectionKey<UiMotionContext> = Symbol('ui-motion')
const transitions = new Set<() => void>()
const transitionListeners = new Set<() => void>()
let disposeInput: (() => void) | undefined

export function finishUiTransitions(): void {
  for (const finish of [...transitions]) finish()
}

/** main 在导航引擎启动前注册，让同一次输入先收稳画面，再计算几何路径。 */
export function installUiMotionInput(): () => void {
  if (!disposeInput) {
    const unsubscribe = inputDispatcher.subscribe(finishUiTransitions)
    const capture = { capture: true }
    window.addEventListener('keydown', finishUiTransitions, capture)
    window.addEventListener('pointerdown', finishUiTransitions, capture)
    disposeInput = () => {
      unsubscribe()
      window.removeEventListener('keydown', finishUiTransitions, capture)
      window.removeEventListener('pointerdown', finishUiTransitions, capture)
      finishUiTransitions()
      disposeInput = undefined
    }
  }
  return () => disposeInput?.()
}

export function trackUiTransition(finish: () => void): () => void {
  transitions.add(finish)
  transitionListeners.forEach(listener => listener())
  return () => {
    transitions.delete(finish)
    transitionListeners.forEach(listener => listener())
  }
}

export function hasUiTransitions(): boolean {
  return transitions.size > 0
}

export function onUiTransitionsChange(listener: () => void): () => void {
  transitionListeners.add(listener)
  return () => transitionListeners.delete(listener)
}

export function useUiMotion(): UiMotionContext | null {
  return inject(motionKey, null)
}

export function useUiMediaMotion(active: () => boolean): void {
  const motion = useUiMotion()
  const owner = Symbol('ui-media')
  watch(active, value => motion?.setMediaActive(owner, value), { immediate: true, flush: 'sync' })
  onDeactivated(() => motion?.setMediaActive(owner, false))
  onBeforeUnmount(() => motion?.setMediaActive(owner, false))
}

export function useUiMotionLifecycle(): void {
  const route = useRoute()
  const preference = window.matchMedia('(prefers-reduced-motion: reduce)')
  const reduced = ref(preference.matches)
  const visible = ref(document.visibilityState !== 'hidden')
  const browserFocused = ref(document.hasFocus())
  const background = ref(inputController.getSnapshot().isBackground)
  const mediaOwners = new Set<symbol>()
  const mediaActive = ref(false)
  const enabled = computed(() => !reduced.value && visible.value
    && (background.value === undefined ? browserFocused.value : !background.value)
    && route.name !== 'xhome-stream' && route.name !== 'xcloud-stream' && !mediaActive.value)
  provide(motionKey, {
    enabled,
    setMediaActive(owner, active) {
      if (active)
        mediaOwners.add(owner)
      else mediaOwners.delete(owner)
      mediaActive.value = mediaOwners.size > 0
    },
  })
  const syncPreference = () => reduced.value = preference.matches
  const syncVisibility = () => visible.value = document.visibilityState !== 'hidden'
  const focus = () => browserFocused.value = true
  const blur = () => browserFocused.value = false
  const disposeGamepad = inputController.subscribe(snapshot => background.value = snapshot.isBackground)
  const disposeMotionInput = installUiMotionInput()
  watchEffect(() => {
    document.documentElement.dataset.uiMotion = enabled.value ? 'active' : 'static'
    if (!enabled.value)
      finishUiTransitions()
  }, { flush: 'sync' })
  onMounted(() => {
    preference.addEventListener('change', syncPreference)
    document.addEventListener('visibilitychange', syncVisibility)
    window.addEventListener('focus', focus)
    window.addEventListener('blur', blur)
  })
  onBeforeUnmount(() => {
    disposeGamepad()
    disposeMotionInput()
    preference.removeEventListener('change', syncPreference)
    document.removeEventListener('visibilitychange', syncVisibility)
    window.removeEventListener('focus', focus)
    window.removeEventListener('blur', blur)
    delete document.documentElement.dataset.uiMotion
  })
}
