<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { Focusable } from '@/navigation/core/vue'
import SettingModalShell from './SettingModalShell.vue'

interface SettingSingleSelectPopupSheetProps {
  open: boolean
  scopeId: string
  title: string
  hint?: string
  options: readonly SettingSelectOptionDefinition[]
  currentValue: string | number | null
  maxListHeight?: string
}

const props = withDefaults(defineProps<SettingSingleSelectPopupSheetProps>(), {
  hint: '',
  maxListHeight: '640px',
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'select', value: string | number): void
}>()

const inputListRef = ref<HTMLElement | null>(null)
let focusObserver: MutationObserver | undefined

const optionNodeId = (index: number): string => `${props.scopeId}.option.${index}`
const defaultFocusId = computed(() => (props.options.length > 0 ? optionNodeId(0) : undefined))

function handleClose(): void {
  emit('close')
}

function scrollFocusedOptionIntoView(): void {
  const listElement = inputListRef.value
  if (listElement === null) {
    return
  }

  const focusedOption = listElement.querySelector<HTMLElement>(
    '.setting-single-select-popup-sheet__option[data-focused="true"]',
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

  if (!props.open || inputListRef.value === null) {
    return
  }

  focusObserver?.disconnect()
  focusObserver = new MutationObserver(() => {
    scrollFocusedOptionIntoView()
  })

  focusObserver.observe(inputListRef.value, {
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
  <SettingModalShell
    :open="props.open"
    :scope-id="props.scopeId"
    :title="props.title"
    :hint="props.hint"
    :default-focus-id="defaultFocusId"
    width="min(100%, 640px)"
    @close="handleClose"
  >
    <div
      ref="inputListRef"
      class="setting-single-select-popup-sheet__list"
      :style="{ '--setting-single-select-popup-sheet-max-height': props.maxListHeight }"
    >
      <Focusable
        v-for="(option, index) in props.options"
        :id="optionNodeId(index)"
        :key="String(option.value)"
        as="button"
        type="button"
        class="setting-single-select-popup-sheet__option"
        :class="{
          'setting-single-select-popup-sheet__option--active': props.currentValue === option.value,
        }"
        :scope-id="props.scopeId"
        :aria-label="option.label"
        :on-back="handleClose"
        @click="emit('select', option.value)"
      >
        <span
          class="setting-single-select-popup-sheet__indicator"
          :class="{
            'setting-single-select-popup-sheet__indicator--active':
              props.currentValue === option.value,
          }"
          aria-hidden="true"
        />

        <span class="setting-single-select-popup-sheet__copy">
          <span class="setting-single-select-popup-sheet__option-title">{{ option.label }}</span>
          <span v-if="option.description" class="setting-single-select-popup-sheet__option-desc">
            {{ option.description }}
          </span>
          <span v-if="option.meta" class="setting-single-select-popup-sheet__option-desc">
            {{ option.meta }}
          </span>
        </span>
      </Focusable>
    </div>
  </SettingModalShell>
</template>

<style scoped>
.setting-single-select-popup-sheet__list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-height: min(
    var(--setting-single-select-popup-sheet-max-height),
    calc(90vh - 220px)
  );
  overflow-y: auto;
  overflow-x: hidden;
  overscroll-behavior: contain;
  /* 左右留白，避免焦点环/box-shadow 被 overflow 裁切 */
  padding: 16px 10px;
}

.setting-single-select-popup-sheet__option {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border: 2px solid transparent;
  border-radius: var(--ui-radius-sm);
  background: var(--color-state-hover);
  text-align: left;
  transition: all var(--ui-motion-fast);
}

.setting-single-select-popup-sheet__option--active {
  background: color-mix(in srgb, var(--brand-primary) 14%, transparent);
}

.setting-single-select-popup-sheet__indicator {
  flex: 0 0 auto;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: transparent;
  border: 2px solid var(--ui-page-text-soft);
}

.setting-single-select-popup-sheet__indicator--active {
  background: var(--brand-primary);
  border-color: var(--brand-primary);
}

.setting-single-select-popup-sheet__copy {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.setting-single-select-popup-sheet__option-title {
  font-size: 16px;
  line-height: 1.2;
  font-weight: 600;
  color: var(--ui-page-text);
}

.setting-single-select-popup-sheet__option-desc {
  font-size: 13px;
  line-height: 1.4;
  color: var(--ui-page-text-soft);
}

.setting-single-select-popup-sheet__option[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-single-select-popup-sheet__option[data-focused='true'] .setting-single-select-popup-sheet__option-title,
.setting-single-select-popup-sheet__option[data-focused='true'] .setting-single-select-popup-sheet__option-desc {
  color: var(--ui-focus-text);
}

.setting-single-select-popup-sheet__option[data-focused='true'] .setting-single-select-popup-sheet__indicator {
  border-color: var(--ui-focus-text);
}
</style>
