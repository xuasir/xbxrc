<script setup lang="ts">
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

interface SettingSingleSelectOption {
  value: string | number
  label: string
  description?: string
  meta?: string
}

interface SettingSingleSelectSheetProps {
  open: boolean
  scopeId: string
  title: string
  hint?: string
  options: readonly SettingSingleSelectOption[]
  currentValue: string | number | null
  maxListHeight?: string
}

const props = withDefaults(defineProps<SettingSingleSelectSheetProps>(), {
  hint: '',
  maxListHeight: '320px',
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'select', value: string | number): void
}>()

const { t } = useI18n()

const listRef = ref<HTMLElement | null>(null)
let focusObserver: MutationObserver | undefined

function handleClose(): void {
  emit('close')
}

function createOptionNodeId(index: number): string {
  return `${props.scopeId}.option.${index}`
}

const idleFocusNodeId = computed(() => `${props.scopeId}.idle`)

const defaultFocusId = computed(() => idleFocusNodeId.value)

function scrollFocusedOptionIntoView(): void {
  const listElement = listRef.value
  if (listElement === null) {
    return
  }

  const focusedOption = listElement.querySelector<HTMLElement>(
    '.setting-single-select-sheet__option[data-focused="true"]',
  )
  if (focusedOption === null) {
    return
  }

  focusedOption.scrollIntoView({
    block: 'nearest',
    inline: 'nearest',
    behavior: 'smooth',
  })
}

async function setupFocusObserver(): Promise<void> {
  await nextTick()

  if (!props.open || listRef.value === null) {
    return
  }

  focusObserver?.disconnect()
  focusObserver = new MutationObserver(() => {
    scrollFocusedOptionIntoView()
  })
  focusObserver.observe(listRef.value, {
    subtree: true,
    attributes: true,
    attributeFilter: ['data-focused'],
  })
  scrollFocusedOptionIntoView()
}

watch(
  () => props.open,
  (open) => {
    if (!open) {
      focusObserver?.disconnect()
      return
    }
    void setupFocusObserver()
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  focusObserver?.disconnect()
  focusObserver = undefined
})
</script>

<template>
  <Transition name="setting-single-select-sheet-transition">
    <div v-if="props.open" class="setting-single-select-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        :active="props.open"
        :trap="true"
        :restore-focus="true"
        :default-focus-id="defaultFocusId"
      >
        <div
          class="setting-single-select-sheet__panel"
          :style="{ '--setting-single-select-sheet-max-height': props.maxListHeight }"
        >
          <Focusable
            :id="idleFocusNodeId"
            as="button"
            type="button"
            class="setting-single-select-sheet__idle-focus"
            :scope-id="props.scopeId"
            :on-back="handleClose"
            :aria-label="t('setting.aria.selectOption')"
          />

          <header class="setting-single-select-sheet__header">
            <div class="setting-single-select-sheet__header-copy">
              <p class="setting-single-select-sheet__eyebrow">{{ t('setting.editor.eyebrow') }}</p>
              <h2 class="setting-single-select-sheet__title">{{ props.title }}</h2>
              <p v-if="props.hint" class="setting-single-select-sheet__hint">{{ props.hint }}</p>
            </div>

            <Focusable
              :id="`${props.scopeId}.close`"
              as="button"
              type="button"
              class="setting-single-select-sheet__close"
              :scope-id="props.scopeId"
              :on-confirm="handleClose"
              :on-back="handleClose"
              :aria-label="t('setting.editor.cancel')"
              @click="handleClose"
            >
              <span class="setting-single-select-sheet__close-icon" aria-hidden="true">✕</span>
            </Focusable>
          </header>

          <div class="setting-single-select-sheet__body">
            <div ref="listRef" class="setting-single-select-sheet__list">
              <Focusable
                v-for="(option, index) in props.options"
                :id="createOptionNodeId(index)"
                :key="String(option.value)"
                as="button"
                type="button"
                class="setting-single-select-sheet__option"
                :class="{
                  'setting-single-select-sheet__option--active': props.currentValue === option.value,
                }"
                :scope-id="props.scopeId"
                :aria-label="option.label"
                :on-confirm="() => emit('select', option.value)"
                :on-back="handleClose"
                @click="emit('select', option.value)"
              >
                <span
                  class="setting-single-select-sheet__indicator"
                  :class="{
                    'setting-single-select-sheet__indicator--active':
                      props.currentValue === option.value,
                  }"
                  aria-hidden="true"
                />

                <span class="setting-single-select-sheet__copy">
                  <span class="setting-single-select-sheet__option-title">{{ option.label }}</span>
                  <span v-if="option.description" class="setting-single-select-sheet__option-desc">
                    {{ option.description }}
                  </span>
                  <span v-if="option.meta" class="setting-single-select-sheet__option-desc">
                    {{ option.meta }}
                  </span>
                </span>
              </Focusable>
            </div>
          </div>
        </div>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.setting-single-select-sheet {
  position: fixed;
  inset: 0;
  z-index: 100;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 40px;
  background: rgba(0, 0, 0, 0.8);
}

.setting-single-select-sheet__panel {
  position: relative;
  width: min(100%, 640px);
  max-height: 85vh;
  display: flex;
  flex-direction: column;
  padding: 0;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 4px;
  background: #1a1a1a;
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.6);
  color: var(--color-text-primary);
  overflow: hidden;
}

.setting-single-select-sheet__idle-focus {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: 0;
  border: 0;
  opacity: 0;
  pointer-events: none;
}

.setting-single-select-sheet__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 24px;
  padding: 32px 32px 16px;
  background: transparent;
}

.setting-single-select-sheet__header-copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.setting-single-select-sheet__close {
  flex: 0 0 auto;
  width: 32px;
  height: 32px;
  border: 0;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.08);
  color: #ffffff;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.setting-single-select-sheet__close[data-focused='true'] {
  background: var(--color-focus-bg);
  color: #ffffff;
  box-shadow: var(--shadow-xbox-focus);
}

.setting-single-select-sheet__close-icon {
  font-size: 16px;
  line-height: 1;
}

.setting-single-select-sheet__eyebrow {
  margin: 0;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--brand-primary);
}

.setting-single-select-sheet__title {
  margin: 0;
  font-size: 28px;
  line-height: 1.2;
  font-weight: 700;
}

.setting-single-select-sheet__hint {
  margin: 4px 0 0;
  font-size: 14px;
  line-height: 1.4;
  color: var(--color-text-secondary);
}

.setting-single-select-sheet__body {
  padding: 0 32px 32px;
  overflow-y: auto;
}

.setting-single-select-sheet__list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-height: var(--setting-single-select-sheet-max-height);
  padding: 16px 0;
}

.setting-single-select-sheet__option {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border: 2px solid transparent;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.04);
  text-align: left;
  transition: all var(--ui-motion-fast);
}

.setting-single-select-sheet__option--active {
  background: rgba(16, 124, 16, 0.15);
}

.setting-single-select-sheet__indicator {
  flex: 0 0 auto;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: transparent;
  border: 2px solid #ffffff;
}

.setting-single-select-sheet__indicator--active {
  background: var(--brand-primary);
  border-color: var(--brand-primary);
}

.setting-single-select-sheet__copy {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.setting-single-select-sheet__option-title {
  font-size: 16px;
  line-height: 1.2;
  font-weight: 600;
}

.setting-single-select-sheet__option-desc {
  font-size: 13px;
  line-height: 1.4;
  color: var(--color-text-secondary);
}

.setting-single-select-sheet__option[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: #ffffff;
  box-shadow: var(--shadow-xbox-focus);
}

.setting-single-select-sheet__option[data-focused='true'] .setting-single-select-sheet__indicator {
  border-color: #ffffff;
}

.setting-single-select-sheet__option[data-focused='true'] .setting-single-select-sheet__indicator--active {
  background: var(--brand-primary);
  border-color: var(--brand-primary);
}

.setting-single-select-sheet__option[data-focused='true'] .setting-single-select-sheet__option-desc {
  color: var(--color-text-secondary);
}

.setting-single-select-sheet-transition-enter-active,
.setting-single-select-sheet-transition-leave-active {
  transition: opacity 300ms var(--ease-standard);
}

.setting-single-select-sheet-transition-enter-from,
.setting-single-select-sheet-transition-leave-to {
  opacity: 0;
}

.setting-single-select-sheet-transition-enter-active .setting-single-select-sheet__panel,
.setting-single-select-sheet-transition-leave-active .setting-single-select-sheet__panel {
  transition: all 400ms var(--ease-standard);
}

.setting-single-select-sheet-transition-enter-from .setting-single-select-sheet__panel {
  opacity: 0;
  transform: scale(0.95);
}

.setting-single-select-sheet-transition-leave-to .setting-single-select-sheet__panel {
  opacity: 0;
  transform: scale(1.02);
}

:global(html[data-ui-density='narrow']) .setting-single-select-sheet__list {
  grid-template-columns: 1fr;
}

:global(html[data-ui-density='narrow']) .setting-single-select-sheet__panel {
  padding: 24px 16px;
}
</style>
