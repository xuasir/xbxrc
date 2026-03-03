<script setup lang="ts">
import type { NodeDef } from '@spatial-navigation/runtime'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { FocusScope, Focusable } from '@spatial-navigation/vue'
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
  maxListHeight: '320px'
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

function handleSelect(value: string | number): void {
  emit('select', value)
}

function createOptionNodeId(index: number): string {
  return `${props.scopeId}.option.${index}`
}

const selectedCandidateNodeId = computed(() => {
  const currentIndex = props.options.findIndex((option) => option.value === props.currentValue)
  const resolvedIndex = currentIndex >= 0 ? currentIndex : 0
  return props.options.length > 0 ? createOptionNodeId(resolvedIndex) : undefined
})

const idleFocusNodeId = computed(() => `${props.scopeId}.idle`)

const defaultFocusId = computed(() => idleFocusNodeId.value)

const idleNeighbors = computed<NodeDef['neighbors']>(() => {
  const candidateNodeId = selectedCandidateNodeId.value
  if (candidateNodeId === undefined) {
    return {}
  }
  return {
    up: candidateNodeId,
    down: candidateNodeId,
    left: candidateNodeId,
    right: candidateNodeId
  }
})

function scrollFocusedOptionIntoView(): void {
  const listElement = listRef.value
  if (listElement === null) {
    return
  }

  const focusedOption = listElement.querySelector<HTMLElement>(
    '.setting-single-select-sheet__option[data-focused="true"]'
  )
  if (focusedOption === null) {
    return
  }

  focusedOption.scrollIntoView({
    block: 'nearest',
    inline: 'nearest',
    behavior: 'smooth'
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
    attributeFilter: ['data-focused']
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
  { immediate: true }
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
            :neighbors="idleNeighbors"
            :on-back="handleClose"
            :aria-label="t('setting.aria.selectOption')"
          />

          <div ref="listRef" class="setting-single-select-sheet__list">
            <Focusable
              v-for="(option, index) in props.options"
              :id="createOptionNodeId(index)"
              :key="String(option.value)"
              as="button"
              type="button"
              class="setting-single-select-sheet__option"
              :class="{
                'setting-single-select-sheet__option--active': props.currentValue === option.value
              }"
              :scope-id="props.scopeId"
              :neighbors="{
                up: index > 0 ? createOptionNodeId(index - 1) : undefined,
                down: index < props.options.length - 1 ? createOptionNodeId(index + 1) : undefined
              }"
              :index="{ order: index }"
              :aria-label="option.label"
              :on-confirm="() => handleSelect(option.value)"
              :on-back="handleClose"
              @click="handleSelect(option.value)"
            >
              <span
                class="setting-single-select-sheet__indicator"
                :class="{
                  'setting-single-select-sheet__indicator--active':
                    props.currentValue === option.value
                }"
                aria-hidden="true"
              ></span>

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
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.setting-single-select-sheet {
  position: fixed;
  inset: 0;
  z-index: 5;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding: var(--ui-settings-modal-padding) var(--ui-settings-modal-padding) 0;
  background:
    linear-gradient(180deg, rgba(8, 10, 18, 0.04), rgba(8, 10, 18, 0.14)),
    color-mix(in srgb, var(--ui-surface-page) 10%, transparent);
  backdrop-filter: blur(2px) saturate(104%);
  -webkit-backdrop-filter: blur(2px) saturate(104%);
}

.setting-single-select-sheet__panel {
  width: min(100%, var(--ui-settings-single-select-width));
  max-width: min(var(--ui-settings-single-select-width), calc(100vw - (var(--ui-settings-modal-padding) * 2)));
  max-height: min(72vh, var(--ui-settings-single-select-max-height));
  padding: 18px 12px 16px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-lg) var(--ui-radius-lg) 0 0;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.06), rgba(255, 255, 255, 0.02)),
    radial-gradient(circle at top right, var(--ui-page-glow-soft), transparent 48%),
    var(--ui-surface-panel-strong);
  box-shadow:
    0 18px 32px rgba(0, 0, 0, 0.24),
    0 0 0 1px rgba(255, 255, 255, 0.02);
  backdrop-filter: blur(12px) saturate(108%);
  -webkit-backdrop-filter: blur(12px) saturate(108%);
  color: var(--ui-page-text);
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

.setting-single-select-sheet__title {
  margin: 0;
  font-size: 15px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  color: var(--ui-page-text);
}

.setting-single-select-sheet__hint {
  margin: 10px 0 0;
  font-size: 12px;
  line-height: 1.45;
  color: var(--ui-page-text-soft);
}

.setting-single-select-sheet__list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-height: var(--setting-single-select-sheet-max-height);
  overflow-y: auto;
  overflow-x: hidden;
  padding-right: 4px;
}

.setting-single-select-sheet__option {
  display: flex;
  align-items: center;
  gap: 14px;
  width: 100%;
  min-height: var(--ui-settings-single-select-option-min-height);
  padding: 8px 12px;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: color-mix(in srgb, var(--ui-surface-panel) 74%, transparent);
  text-align: left;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

.setting-single-select-sheet__option:hover {
  background: color-mix(in srgb, var(--ui-focus-surface) 42%, var(--ui-surface-panel) 58%);
}

.setting-single-select-sheet__option--active {
  border-color: color-mix(in srgb, var(--ui-border-subtle) 80%, transparent);
  background: color-mix(in srgb, var(--ui-focus-surface) 54%, var(--ui-surface-panel) 46%);
}

.setting-single-select-sheet__indicator {
  flex: 0 0 auto;
  width: var(--ui-settings-single-select-indicator-size);
  height: var(--ui-settings-single-select-indicator-size);
  border-radius: 50%;
  background: color-mix(in srgb, var(--ui-page-text-soft) 42%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--ui-page-text) 18%, transparent);
}

.setting-single-select-sheet__indicator--active {
  background: var(--ui-status-positive);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--ui-status-positive) 14%, transparent);
}

.setting-single-select-sheet__copy {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.setting-single-select-sheet__option-title {
  font-size: 12px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-medium);
  color: var(--ui-page-text);
}

.setting-single-select-sheet__option-desc {
  font-size: 11px;
  line-height: 1.3;
  color: var(--ui-page-text-soft);
}

.setting-single-select-sheet__option[data-focused='true'] {
  border-color: var(--ui-border-focus);
  background: color-mix(in srgb, var(--ui-focus-surface) 36%, var(--ui-surface-panel) 64%);
  box-shadow: var(--ui-focus-ring-shadow);
}

.setting-single-select-sheet-transition-enter-active,
.setting-single-select-sheet-transition-leave-active {
  transition: opacity 180ms ease-out;
}

.setting-single-select-sheet-transition-enter-from,
.setting-single-select-sheet-transition-leave-to {
  opacity: 0;
}

.setting-single-select-sheet-transition-enter-active .setting-single-select-sheet__panel,
.setting-single-select-sheet-transition-leave-active .setting-single-select-sheet__panel {
  transition:
    opacity 220ms ease-out,
    transform 220ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.setting-single-select-sheet-transition-enter-from .setting-single-select-sheet__panel,
.setting-single-select-sheet-transition-leave-to .setting-single-select-sheet__panel {
  opacity: 0;
  transform: translateY(28px);
}

:global(html[data-ui-density='compact']) .setting-single-select-sheet__panel,
:global(html[data-ui-density='narrow']) .setting-single-select-sheet__panel {
  padding: 14px 10px 12px;
}

:global(html[data-ui-density='compact']) .setting-single-select-sheet__option,
:global(html[data-ui-density='narrow']) .setting-single-select-sheet__option {
  gap: 12px;
  padding: 7px 10px;
}

:global(html[data-ui-density='compact']) .setting-single-select-sheet__option-desc,
:global(html[data-ui-density='narrow']) .setting-single-select-sheet__option-desc {
  font-size: 10px;
}

:global(html[data-ui-density='narrow']) .setting-single-select-sheet__list {
  gap: 8px;
}

:global(html[data-ui-density='narrow']) .setting-single-select-sheet__option-title {
  font-size: 12px;
}
</style>
