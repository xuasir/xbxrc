<script setup lang="ts">
import { computed, onUnmounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { useShellOverlayHints } from '../layout/shell-context'

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
  busy?: boolean
}

const props = withDefaults(defineProps<SettingModalShellProps>(), {
  hint: '',
  eyebrow: '',
  width: 'min(100%, 640px)',
  maxHeight: '90dvh',
  defaultFocusId: '',
  trap: true,
  restoreFocus: true,
})

const emit = defineEmits<{
  (event: 'close'): void
}>()

const { t } = useI18n()
useShellOverlayHints(() => props.open, () => ({ title: props.title, busy: props.busy }))

const closeNodeId = computed(() => `${props.scopeId}.close`)

function handleClose(): void {
  if (!props.busy)
    emit('close')
}

function handleEscape(event: KeyboardEvent): void {
  if (event.defaultPrevented || !navigationEngine.isTopmostScope(props.scopeId))
    return
  if (event.key !== 'Escape')
    return
  event.preventDefault()
  event.stopImmediatePropagation()
  handleClose()
}
watch(() => props.open, (open) => {
  window.removeEventListener('keydown', handleEscape)
  if (open)
    window.addEventListener('keydown', handleEscape)
}, { immediate: true })
onUnmounted(() => window.removeEventListener('keydown', handleEscape))
</script>

<template>
  <!-- 挂到 body，避免祖先（如设置页 overflow:hidden）裁剪 fixed 全屏遮罩与面板 -->
  <Teleport to="body">
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
              maxHeight: `min(${props.maxHeight}, calc(100dvh - var(--ui-shell-hint-area) - 48px))`,
            }"
            :aria-label="props.title"
            role="dialog" aria-modal="true" :aria-busy="props.busy"
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
                :busy="props.busy"
                @click="handleClose"
              >
                <span class="setting-modal-shell__close-icon" aria-hidden="true">✕</span>
              </Focusable>
            </header>

            <div class="setting-modal-shell__body" data-nav-scroll-container>
              <slot />
            </div>

            <footer v-if="$slots.footer" class="setting-modal-shell__footer">
              <slot name="footer" />
            </footer>
          </section>
        </FocusScope>
      </div>
    </Transition>
  </Teleport>
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
  padding-bottom: calc(var(--ui-shell-hint-area) + var(--ui-settings-modal-padding));
  background: var(--ui-settings-scrim-bg);
  backdrop-filter: blur(var(--ui-settings-scrim-blur));
}

/* Teleport 弹层共用 body；最上层统一提供遮罩，保留背景环境光。 */
.setting-modal-shell:has(~ .setting-modal-shell) {
  background: transparent;
  backdrop-filter: none;
}

/* 下层保留草稿与焦点节点，只呈现当前面板，避免玻璃透出重叠文字。 */
.setting-modal-shell:has(~ .setting-modal-shell) .setting-modal-shell__panel {
  opacity: 0;
}

.setting-modal-shell__panel {
  padding: var(--ui-settings-modal-panel-padding);
  background: var(--ui-settings-glass-overlay-bg);
  --color-focus-bg-strong: var(--ui-material-selected);
  border-radius: var(--ui-radius-panel);
  backdrop-filter: blur(var(--ui-settings-glass-blur));
  box-shadow: var(--ui-shadow-material);
  gap: var(--ui-space-md);
  /* body 独立滚动，标题和操作栏保留外扩焦点阴影。 */
  overflow: visible;
  min-height: 0;
}

.setting-modal-shell__header {
  flex-shrink: 0;
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
  font-weight: 500;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--brand-primary);
}

.setting-modal-shell__title {
  margin: 0;
  font-size: var(--ui-settings-modal-title-size);
  line-height: 1.2;
  font-weight: 500;
  color: var(--ui-page-text);
}

.setting-modal-shell__hint {
  margin: var(--ui-space-xs) 0 0;
  font-size: var(--ui-settings-row-description-size);
  line-height: var(--ui-line-height-relaxed);
  color: var(--color-text-secondary);
}

.setting-modal-shell__close {
  flex: 0 0 auto;
  position: relative;
  width: var(--ui-settings-modal-control-min-height);
  height: var(--ui-settings-modal-control-min-height);
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: transparent;
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
  border-color: transparent;
  z-index: 10;
}

.setting-modal-shell__close:hover { background: var(--ui-material-hover); }

.setting-modal-shell__close-icon {
  font-size: 16px;
  line-height: 1;
}

.setting-modal-shell__body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: var(--ui-space-sm) var(--ui-settings-focus-safe);
  margin-inline: calc(-1 * var(--ui-settings-focus-safe));
  scroll-padding: var(--ui-settings-focus-safe) var(--ui-settings-focus-safe) var(--ui-settings-focus-tail);
}

.setting-modal-shell__footer {
  flex-shrink: 0;
  padding: 0;
  margin: 0;
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
  transform: translateY(8px);
}

.setting-modal-shell-transition-leave-to .setting-modal-shell__panel {
  transform: translateY(8px);
}
@media(prefers-reduced-motion:reduce) { .setting-modal-shell-transition-enter-active, .setting-modal-shell-transition-leave-active, .setting-modal-shell-transition-enter-active .setting-modal-shell__panel, .setting-modal-shell-transition-leave-active .setting-modal-shell__panel { transition: none; } }
</style>
