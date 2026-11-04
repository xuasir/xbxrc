import type { InjectionKey, MaybeRefOrGetter, Ref } from 'vue'
import { computed, inject, onActivated, onDeactivated, onUnmounted, ref, shallowRef, toValue, watchEffect } from 'vue'

export interface ShellPageHints {
  regionLabel?: string
  status?: string
  error?: boolean
}

export interface ShellOverlayHints {
  title: string
  busy?: boolean
}

interface ShellContext {
  page: Ref<ShellPageHints>
  overlay: Readonly<Ref<ShellOverlayHints | undefined>>
  setPage: (owner: symbol, hints?: ShellPageHints) => void
  setOverlay: (owner: symbol, hints?: ShellOverlayHints) => void
}

export const shellContextKey: InjectionKey<ShellContext> = Symbol('app-shell')

/** 页面与弹层只投影当前有效提示，业务状态仍由各自组件拥有。 */
export function createShellContext(): ShellContext {
  const page = shallowRef<ShellPageHints>({})
  const entries = new Map<symbol, ShellOverlayHints>()
  const overlays = shallowRef(entries)
  let pageOwner: symbol | undefined
  return {
    page,
    overlay: computed(() => [...overlays.value.values()].at(-1)),
    setPage(owner, hints) {
      if (hints) {
        pageOwner = owner
        page.value = hints
      }
      else if (pageOwner === owner) {
        pageOwner = undefined
        page.value = {}
      }
    },
    setOverlay(owner, hints) {
      if (hints)
        entries.set(owner, hints)
      else entries.delete(owner)
      overlays.value = new Map(entries)
    },
  }
}

export function useShellPageHints(hints: MaybeRefOrGetter<ShellPageHints>): void {
  const shell = inject(shellContextKey, null)
  const owner = Symbol('page')
  const active = ref(true)
  watchEffect(() => shell?.setPage(owner, active.value ? toValue(hints) : undefined))
  onActivated(() => {
    active.value = true
  })
  onDeactivated(() => {
    active.value = false
  })
  onUnmounted(() => shell?.setPage(owner))
}

export function useShellOverlayHints(open: MaybeRefOrGetter<boolean>, hints: MaybeRefOrGetter<ShellOverlayHints>) {
  const shell = inject(shellContextKey, null)
  const owner = Symbol('overlay')
  watchEffect(() => shell?.setOverlay(owner, toValue(open) ? toValue(hints) : undefined))
  onUnmounted(() => shell?.setOverlay(owner))
  return shell
}
