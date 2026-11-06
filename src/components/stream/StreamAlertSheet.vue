<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable, FocusScope } from '@/navigation/core/vue'

interface StreamAlertAction {
  id: string
  label: string
  danger?: boolean
  primary?: boolean
  disabled?: boolean
}
const props = withDefaults(defineProps<{
  open: boolean
  scopeId: string
  title: string
  body: string
  actions: readonly StreamAlertAction[]
  eyebrow?: string
  help?: string
  feedback?: string
  details?: readonly { label: string, value: string }[]
  clipboardText?: string
  dismissOnBackdrop?: boolean
  busy?: boolean
  resetKey?: number | string
}>(), { dismissOnBackdrop: true })
const emit = defineEmits<{ close: [], select: [id: string] }>()
const { t } = useI18n()
const detailsOpen = ref(false)
const copyState = ref<'idle' | 'copying' | 'copied' | 'failed'>('idle')
let copyGeneration = 0
const defaultFocusId = computed(() => props.actions.find(action => !action.disabled)?.id)
const detailsId = computed(() => `${props.scopeId}.details`)

async function handleClose(): Promise<void> {
  if (detailsOpen.value) {
    detailsOpen.value = false
    await nextTick()
    const node = document.getElementById(detailsId.value)
    if (node)
      navigationEngine.focusElement(node, false)
    return
  }
  emit('close')
}
function handleSelect(id: string): void {
  if (props.actions.find(action => action.id === id && !action.disabled))
    emit('select', id)
}
async function copyDetails(): Promise<void> {
  if (!props.clipboardText || copyState.value === 'copying')
    return
  const generation = ++copyGeneration
  copyState.value = 'copying'
  try {
    await navigator.clipboard.writeText(props.clipboardText)
    if (props.open && generation === copyGeneration)
      copyState.value = 'copied'
  }
  catch {
    if (props.open && generation === copyGeneration)
      copyState.value = 'failed'
  }
}
function handleKeydown(event: KeyboardEvent): void {
  if (event.defaultPrevented || !navigationEngine.isTopmostScope(props.scopeId))
    return
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    void handleClose()
  }
}
watch(() => [props.open, props.resetKey] as const, ([open]) => {
  copyGeneration += 1
  copyState.value = 'idle'
  detailsOpen.value = false
  window.removeEventListener('keydown', handleKeydown, true)
  if (open)
    window.addEventListener('keydown', handleKeydown, true)
}, { immediate: true })
onBeforeUnmount(() => {
  copyGeneration += 1
  window.removeEventListener('keydown', handleKeydown, true)
})
</script>

<template>
  <Transition name="stream-alert-sheet-transition">
    <div v-if="open" class="stream-alert-sheet-layer">
      <div class="stream-alert-sheet-backdrop" @click="dismissOnBackdrop && handleClose()" />
      <FocusScope :id="scopeId" as="section" class="stream-alert-sheet__panel" :active="open" :default-focus-id="defaultFocusId" role="dialog" aria-modal="true" :aria-labelledby="`${scopeId}.title`" :aria-describedby="`${scopeId}.body`" :aria-busy="busy">
        <header class="stream-alert-sheet__header">
          <p v-if="eyebrow" class="stream-alert-sheet__eyebrow">
            {{ eyebrow }}
          </p>
          <h2 :id="`${scopeId}.title`" class="stream-alert-sheet__title">
            <svg class="stream-alert-sheet__status-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="1.7" /><path d="M12 7v6m0 3v.5" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
            {{ title }}
          </h2>
          <p :id="`${scopeId}.body`" class="stream-alert-sheet__body">
            {{ body }}
          </p>
          <p v-if="help" class="stream-alert-sheet__help">
            {{ help }}
          </p>
        </header>
        <div class="stream-alert-sheet__actions">
          <Focusable v-for="action in actions" :id="action.id" :key="action.id" :scope-id="scopeId" as="button" type="button" class="stream-alert-sheet__action" :class="{ 'stream-alert-sheet__action--danger': action.danger, 'stream-alert-sheet__action--primary': action.primary }" :disabled="action.disabled" :on-back="handleClose" @click="handleSelect(action.id)">
            {{ action.label }}
          </Focusable>
        </div>
        <p v-if="feedback" class="stream-alert-sheet__feedback" role="status" aria-live="polite">
          {{ feedback }}
        </p>
        <div v-if="details?.length" class="stream-alert-sheet__details-header">
          <Focusable :id="detailsId" :scope-id="scopeId" as="button" type="button" class="stream-alert-sheet__details-toggle" :aria-expanded="detailsOpen" :aria-controls="`${scopeId}.detail-content`" :on-back="handleClose" @click="detailsOpen = !detailsOpen">
            {{ t('streamPage.failure.details') }} <span aria-hidden="true">{{ detailsOpen ? '⌃' : '⌄' }}</span>
          </Focusable>
        </div>
        <div v-if="detailsOpen" :id="`${scopeId}.detail-content`" class="stream-alert-sheet__details" data-nav-scroll-container>
          <dl>
            <div v-for="row in details" :key="row.label">
              <dt>{{ row.label }}</dt><dd>{{ row.value }}</dd>
            </div>
          </dl>
          <Focusable :id="`${scopeId}.copy`" :scope-id="scopeId" as="button" type="button" class="stream-alert-sheet__action stream-alert-sheet__copy" :disabled="copyState === 'copying'" :on-back="handleClose" @click="copyDetails">
            {{ t('streamPage.failure.copy') }}
          </Focusable>
          <p v-if="copyState !== 'idle'" role="status" aria-live="polite" class="stream-alert-sheet__feedback">
            {{ t(`streamPage.failure.${copyState}`) }}
          </p>
        </div>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.stream-alert-sheet-layer { position: fixed; inset: 0; z-index: var(--z-modal); display: flex; align-items: center; justify-content: center; padding: 24px; }
.stream-alert-sheet-backdrop { position: absolute; inset: 0; background: var(--ui-scrim-bg); backdrop-filter: blur(var(--ui-settings-glass-blur)); }
.stream-alert-sheet__panel { position: relative; width: min(100%, 520px); max-height: calc(100dvh - 48px); padding: 28px; border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-lg); background: var(--ui-settings-glass-overlay-bg, var(--ui-surface-overlay)); color: var(--ui-page-text); display: flex; flex-direction: column; gap: 18px; text-align: left; --color-focus-bg-strong: var(--ui-focus-surface); }
.stream-alert-sheet__header { flex-shrink: 0; }
.stream-alert-sheet__eyebrow { margin: 0 0 6px; color: var(--ui-page-text-soft); font-size: 12px; }
.stream-alert-sheet__title { margin: 0 0 12px; font-size: 23px; font-weight: 700; line-height: 1.4; overflow-wrap: anywhere; }
.stream-alert-sheet__status-icon { width: 24px; height: 24px; margin-right: 8px; vertical-align: -.15em; color: var(--ui-status-warning, var(--ui-page-text-soft)); }
.stream-alert-sheet__body { margin: 0; font-size: 15px; line-height: 1.6; }
.stream-alert-sheet__help { margin: 10px 0 0; font-size: 13px; line-height: 1.6; color: var(--ui-page-text-soft); }
.stream-alert-sheet__actions { display: flex; flex-wrap: wrap; gap: 12px; flex-shrink: 0; }
.stream-alert-sheet__action { --focus-ring-border-width: 2px; flex: 1 1 140px; min-height: 44px; padding: 11px 16px; border: 2px solid transparent; border-radius: var(--ui-radius-md); background: var(--color-state-hover); color: var(--ui-page-text); font: inherit; font-size: 14px; font-weight: 600; transition: background var(--ui-motion-fast); }
.stream-alert-sheet__action--primary { background: var(--brand-primary); color: var(--brand-on-primary); }
.stream-alert-sheet__action--danger { color: var(--ui-status-danger); }
.stream-alert-sheet__action[data-focused='true'], .stream-alert-sheet__details-toggle[data-focused='true'] { background: var(--color-focus-bg-strong); color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 1; }
.stream-alert-sheet__action:disabled { opacity: .5; }
.stream-alert-sheet__details-header { flex-shrink: 0; }
.stream-alert-sheet__details-toggle { --focus-ring-border-width: 2px; min-height: 44px; padding: 8px 12px; border: 2px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: var(--ui-page-text-soft); font: inherit; font-size: 13px; }
.stream-alert-sheet__details { overflow: auto; min-height: 0; border-top: 1px solid var(--ui-border-subtle); padding: 16px 12px 28px; margin-inline: -12px; scroll-padding: 12px 12px 28px; }
.stream-alert-sheet__details dl { margin: 0 0 16px; display: grid; gap: 10px; font-size: 12px; line-height: 1.5; }
.stream-alert-sheet__details dl > div { display: grid; grid-template-columns: minmax(80px, 1fr) minmax(0, 2fr); gap: 12px; }
.stream-alert-sheet__details dt { color: var(--ui-page-text-soft); }
.stream-alert-sheet__details dd { margin: 0; overflow-wrap: anywhere; }
.stream-alert-sheet__copy { flex: none; }
.stream-alert-sheet__feedback { margin: 0; color: var(--ui-page-text-soft); font-size: 13px; line-height: 1.5; }
.stream-alert-sheet-transition-enter-active, .stream-alert-sheet-transition-leave-active { transition: opacity 200ms ease; }
.stream-alert-sheet-transition-enter-from, .stream-alert-sheet-transition-leave-to { opacity: 0; }
@media (max-width: 480px) { .stream-alert-sheet-layer { padding: 16px; } .stream-alert-sheet__panel { padding: 22px 18px; max-height: calc(100dvh - 32px); } .stream-alert-sheet__actions { flex-direction: column; } .stream-alert-sheet__action { flex-basis: auto; width: 100%; } }
@media (max-height: 460px) { .stream-alert-sheet__panel { overflow: auto; } }
@media (prefers-reduced-motion: reduce) { .stream-alert-sheet-transition-enter-active, .stream-alert-sheet-transition-leave-active { transition: none; } }
</style>
