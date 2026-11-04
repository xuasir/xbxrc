<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { FOCUSABLE_SELECTOR, navigationEngine } from '@/navigation/core/engine'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import exitIcon from '../../assets/nav/exit.svg'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../../navigation/spatial-nav.constants'
import BrandedLoading from '../common/BrandedLoading.vue'
import { useShellOverlayHints } from '../layout/shell-context'
import SettingConfirmSheet from '../settings/SettingConfirmSheet.vue'

interface UserProfileMenuProps {
  displayName: string
  secondaryName: string
  score: string
  statusText: string
  avatarUrl?: string
  open: boolean
  loggingOut?: boolean
}

const props = withDefaults(defineProps<UserProfileMenuProps>(), {
  avatarUrl: '',
  loggingOut: false,
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'logout'): void
}>()

const { t } = useI18n()
const logoutConfirm = ref(false)
useShellOverlayHints(() => props.open, () => ({ title: t('shell.account'), busy: props.loggingOut }))
function handleKeydown(event: KeyboardEvent) {
  if (event.defaultPrevented || !navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.userMenu))
    return
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    emitClose()
  }
  else if (event.key === 'Tab') {
    const nodes = [...(document.getElementById('user-menu-panel')?.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR) ?? [])]
    const current = nodes.indexOf(document.activeElement as HTMLElement)
    const next = nodes[(current + (event.shiftKey ? -1 : 1) + nodes.length) % nodes.length]
    if (next) {
      event.preventDefault()
      navigationEngine.focusElement(next, false)
    }
  }
}
watch(() => props.open, (open) => {
  window.removeEventListener('keydown', handleKeydown)
  if (open) {
    window.addEventListener('keydown', handleKeydown)
  }
  else {
    logoutConfirm.value = false
    navigationEngine.updateActiveScope(SPATIAL_NAV_SCOPE_IDS.userMenu, false)
  }
}, { immediate: true })
onUnmounted(() => window.removeEventListener('keydown', handleKeydown))

function emitClose(): void {
  if (!props.loggingOut)
    emit('close')
}

function emitLogout(): void {
  emit('logout')
}
</script>

<template>
  <Teleport to="body">
    <Transition name="user-menu-transition">
      <div v-if="props.open" class="user-menu-layer">
        <button
          type="button"
          class="user-menu-layer__backdrop"
          :aria-label="t('userMenu.close')"
          tabindex="-1"
          @click="emitClose"
        />

        <div class="user-menu-anchor">
          <FocusScope
            :id="SPATIAL_NAV_SCOPE_IDS.userMenu"
            as="section"
            class="user-menu__panel"
            :aria-label="t('shell.profile')"
            :active="props.open"
            :default-focus-id="SPATIAL_NAV_NODE_IDS.userMenu.logout"
          >
            <section id="user-menu-panel" role="dialog" aria-modal="true" :aria-label="t('shell.profile')" :aria-busy="props.loggingOut">
              <Focusable
                id="user-menu.close"
                as="button"
                type="button"
                class="user-menu__close"
                :on-back="emitClose"
                :disabled="props.loggingOut"
                :aria-label="t('userMenu.close')"
                @click="emitClose"
              >
                <span class="user-menu__close-line user-menu__close-line--first" aria-hidden="true" />
                <span class="user-menu__close-line user-menu__close-line--second" aria-hidden="true" />
              </Focusable>

              <div class="user-menu__block user-menu__block--info">
                <div class="user-menu__avatar-wrap">
                  <img
                    v-if="props.avatarUrl"
                    class="user-menu__avatar"
                    :src="props.avatarUrl"
                    alt=""
                  >
                  <span
                    v-else
                    class="user-menu__avatar user-menu__avatar--placeholder"
                    aria-hidden="true"
                  >
                    {{ props.displayName.slice(0, 1).toUpperCase() }}
                  </span>
                  <span class="user-menu__avatar-online" aria-hidden="true" />
                </div>

                <div class="user-menu__identity">
                  <div class="user-menu__name-line">
                    <p class="user-menu__display-name">
                      {{ props.displayName }}
                    </p>
                    <p v-if="props.secondaryName" class="user-menu__secondary-name">
                      {{ props.secondaryName }}
                    </p>
                  </div>

                  <div class="user-menu__score-row">
                    <span class="user-menu__score-badge" aria-hidden="true">G</span>
                    <p class="user-menu__score">
                      {{ t('userMenu.score', { value: props.score }) }}
                    </p>
                  </div>
                </div>
              </div>

              <div class="user-menu__block user-menu__status">
                <span class="user-menu__status-indicator" aria-hidden="true" />
                <div class="user-menu__status-copy">
                  <p class="user-menu__status-value">
                    {{ props.statusText }}
                  </p>
                </div>
              </div>

              <div class="user-menu__divider" aria-hidden="true" />

              <div class="user-menu__actions">
                <Focusable
                  :id="SPATIAL_NAV_NODE_IDS.userMenu.logout"
                  as="button"
                  type="button"
                  class="user-menu__logout"
                  :disabled="props.loggingOut"
                  :on-back="emitClose"
                  @click="logoutConfirm = true"
                >
                  <BrandedLoading v-if="props.loggingOut" size="xs" />
                  <img v-else class="user-menu__logout-icon" :src="exitIcon" alt="" aria-hidden="true">
                  <span class="user-menu__logout-label">{{ t('userMenu.logout') }}</span>
                </Focusable>
              </div>
            </section>
          </FocusScope>
        </div>
      </div>
    </Transition>
  </Teleport>
  <SettingConfirmSheet :open="logoutConfirm" scope-id="user-menu.logout-confirm" :eyebrow="t('shell.account')" :title="t('userMenu.logout')" :message="t('userMenu.confirmLogout')" :confirm-label="t('userMenu.logout')" :busy="props.loggingOut" @close="logoutConfirm = false" @confirm="emitLogout" />
</template>

<style scoped>
.user-menu-layer { position: fixed; inset: 0; z-index: var(--z-overlay); }
.user-menu-layer__backdrop { position: absolute; inset: 0; border: 0; background: color-mix(in srgb, var(--ui-page-bg) 30%, transparent); cursor: default; }
.user-menu-anchor { position: absolute; top: calc(var(--ui-shell-header-height) + 8px); right: var(--ui-shell-edge-inset); pointer-events: none; }
.user-menu__panel { width: min(calc(100vw - 32px), 320px); max-height: calc(100dvh - var(--ui-shell-header-height) - var(--ui-shell-hint-area) - 24px); overflow: auto; padding: var(--space-24); position: relative; pointer-events: auto; border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-panel); background: var(--ui-material-overlay); backdrop-filter: blur(24px) saturate(125%); box-shadow: inset 0 1px 0 var(--ui-material-edge), var(--ui-shadow-material); color: var(--ui-page-text); }
.user-menu__close { position: absolute; top: var(--space-12); right: var(--space-12); width: 32px; height: 32px; border: 1px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: inherit; display: grid; place-items: center; cursor: pointer; }
.user-menu__close-line { position: absolute; width: 14px; height: 1px; background: currentColor; }
.user-menu__close-line--first { transform: rotate(45deg); }
.user-menu__close-line--second { transform: rotate(-45deg); }
.user-menu__block--info { display: flex; align-items: center; gap: var(--space-16); padding-top: var(--space-24); }
.user-menu__avatar-wrap { flex-shrink: 0; position: relative; width: 56px; height: 56px; }
.user-menu__avatar { width: 100%; height: 100%; border-radius: 50%; object-fit: cover; border: 1px solid var(--ui-border-subtle); }
.user-menu__avatar--placeholder { display: grid; place-items: center; background: var(--ui-material-selected); font-size: 24px; }
.user-menu__avatar-online { position: absolute; right: 1px; bottom: 1px; width: 10px; height: 10px; border: 2px solid var(--ui-panel-top); border-radius: 50%; background: var(--ui-status-positive); }
.user-menu__identity { min-width: 0; overflow-wrap: anywhere; }
.user-menu__display-name { font-size: 18px; line-height: 24px; font-weight: 500; }
.user-menu__secondary-name { font-size: 12px; line-height: 18px; color: var(--color-text-secondary); }
.user-menu__score-row { display: flex; align-items: center; gap: var(--space-8); margin-top: var(--space-8); }
.user-menu__score-badge { display: grid; place-items: center; width: 16px; height: 16px; border-radius: 50%; color: var(--ui-icon-text); border: 1px solid var(--ui-border-hover); font-size: 10px; }
.user-menu__score { font-size: 12px; color: var(--color-text-secondary); }
.user-menu__status { display: flex; align-items: center; gap: var(--space-8); margin-top: var(--space-24); font-size: 12px; color: var(--color-text-secondary); }
.user-menu__status-indicator { width: 6px; height: 6px; border-radius: 50%; background: var(--ui-status-positive); }
.user-menu__divider { height: 1px; margin: var(--space-16) 0; background: var(--ui-border-subtle); }
.user-menu__logout { display: flex; align-items: center; gap: var(--space-12); min-height: 44px; width: 100%; padding: var(--space-12); border: 1px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: var(--color-text-primary); text-align: left; font: inherit; font-size: 14px; cursor: pointer; }
.user-menu__logout-icon { width: 18px; height: 18px; filter: var(--ui-nav-icon-filter); }
.user-menu__close:hover, .user-menu__logout:hover { background: var(--ui-material-hover); }
.user-menu__close[data-focused='true'], .user-menu__logout[data-focused='true'] { background: var(--ui-material-selected); box-shadow: var(--shadow-xbox-focus); }
.user-menu-transition-enter-active, .user-menu-transition-leave-active { transition: opacity 160ms var(--ease-standard); }
.user-menu-transition-enter-active .user-menu__panel, .user-menu-transition-leave-active .user-menu__panel { transition: transform 160ms var(--ease-standard); }
.user-menu-transition-enter-from, .user-menu-transition-leave-to { opacity: 0; }
.user-menu-transition-enter-from .user-menu__panel, .user-menu-transition-leave-to .user-menu__panel { transform: translateY(-8px); }
@media(max-width:700px) { .user-menu-anchor { right: 16px; } }
@media(prefers-reduced-motion:reduce) { .user-menu-transition-enter-active, .user-menu-transition-leave-active, .user-menu-transition-enter-active .user-menu__panel, .user-menu-transition-leave-active .user-menu__panel { transition: none; } }
</style>
