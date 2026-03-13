<script setup lang="ts">
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { useI18n } from 'vue-i18n'
import exitIcon from '../../assets/nav/exit.svg'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../../navigation/spatial-nav.constants'

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

function emitClose(): void {
  emit('close')
}

function emitLogout(): void {
  emit('logout')
}
</script>

<template>
  <Transition name="user-menu-transition">
    <div v-if="props.open" class="user-menu-layer">
      <button
        type="button"
        class="user-menu-layer__backdrop"
        :aria-label="t('userMenu.close')"
        @click="emitClose"
      />

      <div class="user-menu-anchor">
        <FocusScope
          :id="SPATIAL_NAV_SCOPE_IDS.userMenu"
          as="section"
          class="user-menu__panel"
          aria-label="User menu"
          :active="props.open"
          :default-focus-id="SPATIAL_NAV_NODE_IDS.userMenu.logout"
        >
          <Focusable
            id="user-menu.close"
            as="button"
            type="button"
            class="user-menu__close"
            :on-confirm="emitClose"
            :on-back="emitClose"
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
                alt="User avatar"
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
              :on-confirm="emitLogout"
              :on-back="emitClose"
              @click="emitLogout"
            >
              <img class="user-menu__logout-icon" :src="exitIcon" alt="" aria-hidden="true">
              <span class="user-menu__logout-label">{{ t('userMenu.logout') }}</span>
            </Focusable>
          </div>
        </FocusScope>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.user-menu-layer {
  position: fixed;
  inset: 0;
  z-index: var(--z-overlay);
}

.user-menu-layer__backdrop {
  position: absolute;
  inset: 0;
  border: 0;
  background: var(--ui-scrim-bg);
  backdrop-filter: blur(4px);
  cursor: default;
}

.user-menu-anchor {
  position: absolute;
  top: 24px;
  right: 24px;
  bottom: 24px;
  pointer-events: none;
  display: flex;
  align-items: stretch;
}

.user-menu__panel {
  width: min(calc(100vw - 48px), 340px);
  pointer-events: auto;
  position: relative;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 16px;
  background: var(--ui-surface-overlay);
  box-shadow: var(--ui-shadow-overlay);
  color: var(--ui-page-text);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 24px 16px;
}

.user-menu__close {
  position: absolute;
  top: 16px;
  right: 16px;
  width: 36px;
  height: 36px;
  border: 0;
  border-radius: var(--ui-radius-pill);
  background: transparent;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--ui-motion-fast);
  z-index: 10;
}

.user-menu__close[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.user-menu__close-line {
  position: absolute;
  width: 16px;
  height: 2px;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-page-text);
}

.user-menu__close[data-focused='true'] .user-menu__close-line {
  background: var(--ui-focus-text);
}

.user-menu__close-line--first {
  transform: rotate(45deg);
}

.user-menu__close-line--second {
  transform: rotate(-45deg);
}

.user-menu__block--info {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  padding: 32px 0 16px;
  text-align: center;
}

.user-menu__avatar-wrap {
  position: relative;
  width: 88px;
  height: 88px;
}

.user-menu__avatar {
  width: 88px;
  height: 88px;
  border-radius: var(--ui-radius-pill);
  object-fit: cover;
  border: 3px solid var(--ui-border-subtle);
}

.user-menu__avatar--placeholder {
  background: var(--color-state-hover);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 36px;
  font-weight: 800;
}

.user-menu__avatar-online {
  position: absolute;
  right: 6px;
  bottom: 6px;
  width: 16px;
  height: 16px;
  border: 3px solid var(--ui-surface-overlay);
  border-radius: var(--ui-radius-pill);
  background: var(--ui-status-positive);
}

.user-menu__display-name {
  font-size: 24px;
  line-height: 1.2;
  font-weight: 800;
  letter-spacing: -0.01em;
}

.user-menu__secondary-name {
  color: var(--ui-page-text-soft);
  font-size: 15px;
  margin-top: 2px;
}

.user-menu__score-row {
  margin-top: 12px;
  display: flex;
  align-items: center;
  gap: 8px;
  background: var(--color-state-hover);
  padding: 6px 16px;
  border-radius: 999px;
}

.user-menu__score-badge {
  width: 18px;
  height: 18px;
  border-radius: var(--ui-radius-pill);
  background: var(--brand-primary);
  color: #ffffff;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 900;
}

.user-menu__score {
  font-size: 15px;
  font-weight: 700;
}

.user-menu__status {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  margin-top: 8px;
}

.user-menu__status-indicator {
  width: 8px;
  height: 8px;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-status-positive);
}

.user-menu__status-value {
  font-size: 13px;
  font-weight: 600;
  color: var(--ui-page-text-soft);
}

.user-menu__divider {
  height: 1px;
  margin: 24px 0;
  background: var(--ui-border-subtle);
}

.user-menu__actions {
  margin-top: auto;
  display: flex;
  flex-direction: column;
}

.user-menu__logout {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 16px 20px;
  border: 2px solid transparent;
  border-radius: 12px;
  background: var(--color-state-hover);
  color: var(--ui-page-text);
  cursor: pointer;
  transition: all var(--ui-motion-fast);
  text-align: left;
}

.user-menu__logout[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.user-menu__logout-icon {
  width: 20px;
  height: 20px;
  filter: var(--ui-nav-icon-filter);
}

.user-menu__logout-label {
  font-size: 17px;
  font-weight: 700;
}

.user-menu-transition-enter-active,
.user-menu-transition-leave-active {
  transition: opacity 250ms ease;
}

.user-menu-transition-enter-active .user-menu__panel,
.user-menu-transition-leave-active .user-menu__panel {
  transition: transform 350ms cubic-bezier(0.2, 0, 0, 1);
}

.user-menu-transition-enter-from .user-menu__panel {
  transform: translateX(calc(100% + 48px));
}

.user-menu-transition-leave-to .user-menu__panel {
  transform: translateX(calc(100% + 48px));
}

.user-menu-transition-enter-from,
.user-menu-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .user-menu-anchor {
  left: 24px;
}

:global(html[data-ui-density='narrow']) .user-menu__panel {
  width: 100%;
}
</style>
