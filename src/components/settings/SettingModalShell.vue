<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable, FocusScope } from '@/navigation/core/vue'

interface SettingModalShellProps {
  open: boolean
  scopeId: string
  title: string
  hint?: string
  eyebrow?: string
  width?: string
  maxHeight?: string
  defaultFocusId?: string
  trap?: boolean
  restoreFocus?: boolean
}

const props = withDefaults(defineProps<SettingModalShellProps>(), {
  hint: '',
  eyebrow: '',
  width: 'min(100%, 640px)',
  maxHeight: '90vh',
  defaultFocusId: '',
  trap: true,
  restoreFocus: true,
})

const emit = defineEmits<{
  (event: 'close'): void
}>()

const { t } = useI18n()

const closeNodeId = computed(() => `${props.scopeId}.close`)

function handleClose(): void {
  emit('close')
}
</script>

<template>
  <Transition name="setting-modal-shell-transition">
    <div v-if="props.open" class="setting-modal-shell" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        :active="props.open"
        :trap="props.trap"
        :restore-focus="props.restoreFocus"
        :default-focus-id="props.defaultFocusId || undefined"
      >
        <section
          class="setting-modal-shell__panel ui-overlay-panel"
          :style="{
            width: props.width,
            maxHeight: props.maxHeight,
          }"
          :aria-label="props.title"
        >
          <header class="setting-modal-shell__header">
            <div class="setting-modal-shell__header-copy">
              <p v-if="props.eyebrow" class="setting-modal-shell__eyebrow">
                {{ props.eyebrow }}
              </p>
              <p v-else class="setting-modal-shell__eyebrow">
                {{ t('setting.editor.eyebrow') }}
              </p>

              <h2 class="setting-modal-shell__title">
                {{ props.title }}
              </h2>

              <p v-if="props.hint" class="setting-modal-shell__hint">
                {{ props.hint }}
              </p>

              <slot name="headerExtra" />
            </div>

            <Focusable
              :id="closeNodeId"
              as="button"
              type="button"
              class="setting-modal-shell__close"
              :scope-id="props.scopeId"
              :on-back="handleClose"
              :aria-label="t('setting.editor.cancel')"
              @click="handleClose"
            >
              <span class="setting-modal-shell__close-icon" aria-hidden="true">✕</span>
            </Focusable>
          </header>

          <div class="setting-modal-shell__body">
            <slot />
          </div>

          <footer v-if="$slots.footer" class="setting-modal-shell__footer">
            <slot name="footer" />
          </footer>
        </section>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.setting-modal-shell {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ui-settings-modal-padding);
  background: var(--ui-scrim-bg);
}

.setting-modal-shell__panel {
  padding: var(--ui-settings-modal-panel-padding);
  gap: var(--ui-space-lg);
  overflow: hidden;
}

.setting-modal-shell__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--ui-space-lg);
}

.setting-modal-shell__header-copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.setting-modal-shell__eyebrow {
  margin: 0;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--brand-primary);
}

.setting-modal-shell__title {
  margin: 0;
  font-size: var(--ui-settings-modal-title-size);
  line-height: 1.2;
  font-weight: 700;
  color: var(--ui-page-text);
}

.setting-modal-shell__hint {
  margin: 4px 0 0;
  font-size: 14px;
  line-height: 1.4;
  color: var(--color-text-secondary);
}

.setting-modal-shell__close {
  flex: 0 0 auto;
  width: 32px;
  height: 32px;
  border: 0;
  border-radius: 50%;
  background: var(--color-state-hover);
  color: var(--ui-page-text);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.setting-modal-shell__close[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-modal-shell__close-icon {
  font-size: 16px;
  line-height: 1;
}

.setting-modal-shell__body {
  flex: 1 1 auto;
  min-height: 0;
  overflow: hidden;
}

.setting-modal-shell__footer {
  padding-top: var(--ui-space-md);
  border-top: 1px solid rgba(255, 255, 255, 0.05);
}

/* 动画：与现有 setting-*sheet 统一为淡入 + panel scale */
.setting-modal-shell-transition-enter-active,
.setting-modal-shell-transition-leave-active {
  transition: opacity 300ms var(--ease-standard);
}

.setting-modal-shell-transition-enter-from,
.setting-modal-shell-transition-leave-to {
  opacity: 0;
}

.setting-modal-shell-transition-enter-active .setting-modal-shell__panel,
.setting-modal-shell-transition-leave-active .setting-modal-shell__panel {
  transition: transform 400ms var(--ease-standard);
}

.setting-modal-shell-transition-enter-from .setting-modal-shell__panel {
  transform: scale(0.95);
}

.setting-modal-shell-transition-leave-to .setting-modal-shell__panel {
  transform: scale(1.02);
}
</style>

