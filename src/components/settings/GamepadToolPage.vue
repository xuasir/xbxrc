<script setup lang="ts">
import { onActivated, onBeforeUnmount, onDeactivated, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { useShellPageHints } from '../layout/shell-context'

const props = defineProps<{ scopeId: string, title: string, hint?: string, defaultFocusId?: string, locked?: boolean, busy?: boolean, pointerBack?: boolean }>()
const emit = defineEmits<{ back: [], pointerBack: [] }>()
const { t } = useI18n()
let pageActive = true
useShellPageHints(() => ({ status: props.title, exitHint: props.locked ? `${t('gamepadSettings.lockedExit')} · Ctrl + Shift + Esc` : undefined, busy: props.busy }))
function back(): void {
  if (!props.locked && !props.busy)
    emit('back')
}
function escape(event: KeyboardEvent): void {
  if (!pageActive || event.defaultPrevented || event.key !== 'Escape' || !navigationEngine.isTopmostScope(props.scopeId))
    return
  // 锁定会话负责组合出口；普通返回由当前子页消费。
  if (props.locked && event.ctrlKey && event.shiftKey)
    return
  event.preventDefault()
  event.stopImmediatePropagation()
  back()
}
onMounted(() => window.addEventListener('keydown', escape, true))
onActivated(() => {
  pageActive = true
})
onDeactivated(() => {
  pageActive = false
})
onBeforeUnmount(() => window.removeEventListener('keydown', escape, true))
</script>

<template>
  <FocusScope :id="scopeId" :default-focus-id="defaultFocusId" :restore-focus="false">
    <section class="gamepad-tool-page" data-gamepad-page :data-input-locked="locked || undefined" :aria-label="title" :aria-busy="busy" @nav-back.prevent="back">
      <header class="gamepad-tool-page__header">
        <div class="gamepad-tool-page__heading">
          <button v-if="pointerBack" :id="`${scopeId}.touchBack`" class="gamepad-tool-page__back" data-pointer-only tabindex="-1" :aria-label="t('gamepadSettings.backToSettings')" :disabled="busy" @keydown.prevent.stop @click="emit('pointerBack')">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>
          </button>
          <Focusable v-if="!locked && !pointerBack" :id="`${scopeId}.back`" as="button" :scope-id="scopeId" class="gamepad-tool-page__back" :busy="busy" :aria-label="t('gamepadSettings.backToSettings')" @click="back">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>
          </Focusable>
          <div>
            <p class="gamepad-tool-page__eyebrow">
              {{ t('controllerPage.settings') }}
            </p>
            <div class="gamepad-tool-page__title-row">
              <h1>{{ title }}</h1><slot name="title-aside" />
            </div>
          </div>
        </div>
        <div v-if="$slots.actions && !locked" class="gamepad-tool-page__actions">
          <slot name="actions" />
        </div>
      </header>
      <div class="gamepad-tool-page__body" data-nav-scroll-container>
        <p v-if="hint" class="gamepad-tool-page__hint">
          {{ hint }}
        </p>
        <slot />
      </div>
    </section>
  </FocusScope>
</template>

<style scoped>
.gamepad-tool-page { display: grid; grid-template-rows: auto minmax(0, 1fr); width: 100%; height: 100%; min-height: 0; min-width: 0; }
.gamepad-tool-page__header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--space-12); padding: var(--space-8) var(--space-12) var(--space-16); }
.gamepad-tool-page__heading { display: flex; align-items: center; gap: var(--space-12); }
.gamepad-tool-page__title-row { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-8); min-width: 0; }
.gamepad-tool-page__eyebrow { margin: 0 0 var(--space-4); font-size: 12px; color: var(--color-text-secondary); }
.gamepad-tool-page h1 { margin: 0; font-size: var(--ui-text-title-md); line-height: var(--ui-line-height-default); font-weight: var(--ui-font-weight-medium); }
.gamepad-tool-page__actions { margin-left: auto; display: flex; align-items: center; gap: var(--space-8); }
.gamepad-tool-page__back { display: grid; place-items: center; width: 40px; height: 40px; border: 1px solid transparent; border-radius: var(--ui-radius-md); background: var(--ui-settings-item-bg); color: var(--color-text-primary); }
.gamepad-tool-page__back svg { width: 22px; height: 22px; }
.gamepad-tool-page__back:hover { background: var(--ui-material-hover); }
.gamepad-tool-page__back[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.gamepad-tool-page__body { min-width: 0; min-height: 0; overflow: auto; padding: var(--ui-settings-focus-safe) var(--space-12) var(--ui-settings-focus-tail); scroll-padding: var(--ui-settings-focus-safe); }
.gamepad-tool-page__hint { margin: 0 0 var(--space-16); font-size: var(--ui-text-body-sm); color: var(--color-text-secondary); line-height: var(--ui-line-height-relaxed); }
@media(max-height:700px) { .gamepad-tool-page__header { padding-bottom: var(--space-12); } }
</style>
