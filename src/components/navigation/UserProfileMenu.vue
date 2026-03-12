<script setup lang="ts">
import type { NodeDef } from '@spatial-navigation/runtime'
import { Focusable, FocusScope } from '@spatial-navigation/vue'
import { useI18n } from 'vue-i18n'
import exitIcon from '../../assets/nav/exit.svg'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../../navigation/spatial-nav.constants'

interface UserProfileMenuProps {
  scopeId: string
  logoutUpNeighborId?: string
  displayName: string
  secondaryName: string
  score: string
  statusText: string
  avatarUrl?: string
  open: boolean
  loggingOut?: boolean
}

const props = withDefaults(defineProps<UserProfileMenuProps>(), {
  logoutUpNeighborId: '',
  avatarUrl: '',
  loggingOut: false,
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'logout'): void
}>()

const { t } = useI18n()

function buildNeighbors(options: { up?: string }): NodeDef['neighbors'] {
  const neighbors: NodeDef['neighbors'] = {}
  if (options.up !== undefined && options.up !== '') {
    neighbors.up = options.up
  }
  return neighbors
}

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
          :active="props.open"
          :restore-focus="true"
          :trap="true"
          :default-focus-id="SPATIAL_NAV_NODE_IDS.userMenu.idle"
        >
          <section class="user-menu__panel" aria-label="User menu">
            <Focusable
              :id="SPATIAL_NAV_NODE_IDS.userMenu.idle"
              as="button"
              type="button"
              class="user-menu__idle-focus"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.userMenu"
              :neighbors="{
                up: 'user-menu.close',
                down: SPATIAL_NAV_NODE_IDS.userMenu.logout,
              }"
              :aria-label="t('userMenu.close')"
            />

            <Focusable
              id="user-menu.close"
              as="button"
              type="button"
              class="user-menu__close"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.userMenu"
              :neighbors="{ down: SPATIAL_NAV_NODE_IDS.userMenu.logout }"
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

            <Focusable
              :id="SPATIAL_NAV_NODE_IDS.userMenu.logout"
              as="button"
              type="button"
              class="user-menu__block user-menu__logout"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.userMenu"
              :neighbors="buildNeighbors({ up: props.logoutUpNeighborId })"
              :disabled="props.loggingOut"
              :on-confirm="emitLogout"
              :on-back="emitClose"
              @click="emitLogout"
            >
              <img class="user-menu__logout-icon" :src="exitIcon" alt="" aria-hidden="true">
              <span class="user-menu__logout-label">{{ t('userMenu.logout') }}</span>
            </Focusable>
          </section>
        </FocusScope>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.user-menu-layer {
  position: absolute;
  inset: 0;
  z-index: 4;
}

.user-menu-layer__backdrop {
  position: absolute;
  inset: 0;
  border: 0;
  background: rgba(0, 0, 0, 0.4);
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
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 12px;
  background: #252423;
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.6);
  color: var(--ui-page-text);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.user-menu__idle-focus {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: 0;
  border: 0;
  opacity: 0;
  pointer-events: none;
}

.user-menu__close {
  position: absolute;
  top: 24px;
  right: 24px;
  width: 32px;
  height: 32px;
  border: 0;
  border-radius: var(--ui-radius-pill);
  background: transparent;
  cursor: pointer;
  opacity: 0.82;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--ui-motion-fast);
}

.user-menu__close[data-focused='true'] {
  background: rgba(255, 255, 255, 0.1);
  box-shadow: var(--shadow-xbox-focus);
  opacity: 1;
}

.user-menu__close-line {
  position: absolute;
  width: 16px;
  height: 2px;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-page-text);
}

.user-menu__close-line--first {
  transform: rotate(45deg);
}

.user-menu__close-line--second {
  transform: rotate(-45deg);
}

.user-menu__block {
  width: 100%;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: inherit;
}

.user-menu__block--info {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  padding: 24px 0;
  text-align: center;
}

.user-menu__avatar-wrap {
  position: relative;
  width: 80px;
  height: 80px;
}

.user-menu__avatar {
  width: 80px;
  height: 80px;
  border-radius: var(--ui-radius-pill);
  object-fit: cover;
  border: 2px solid rgba(255, 255, 255, 0.1);
}

.user-menu__avatar--placeholder {
  background: #3a3a3a;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 32px;
  font-weight: var(--ui-font-weight-bold);
}

.user-menu__avatar-online {
  position: absolute;
  right: 4px;
  bottom: 4px;
  width: 14px;
  height: 14px;
  border: 2px solid #252423;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-status-positive);
}

.user-menu__identity {
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
}

.user-menu__name-line {
  display: flex;
  flex-direction: column;
  align-items: center;
}

.user-menu__display-name {
  font-size: 22px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  letter-spacing: 0.01em;
}

.user-menu__secondary-name {
  color: var(--ui-page-text-soft);
  font-size: 14px;
  font-weight: var(--ui-font-weight-medium);
}

.user-menu__score-row {
  margin-top: 8px;
  display: flex;
  align-items: center;
  gap: 8px;
  background: rgba(255, 255, 255, 0.06);
  padding: 4px 12px;
  border-radius: 999px;
}

.user-menu__score-badge {
  width: 18px;
  height: 18px;
  border-radius: var(--ui-radius-pill);
  background: #ffffff;
  color: #000000;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 900;
}

.user-menu__score {
  font-size: 14px;
  font-weight: var(--ui-font-weight-bold);
  color: var(--ui-page-text);
}

.user-menu__status {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  margin-top: 12px;
}

.user-menu__status-indicator {
  width: 8px;
  height: 8px;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-status-positive);
}

.user-menu__status-value {
  font-size: 12px;
  font-weight: var(--ui-font-weight-semibold);
  color: var(--ui-page-text-soft);
}

.user-menu__divider {
  height: 1px;
  margin: 24px 0;
  background: rgba(255, 255, 255, 0.08);
}

.user-menu__logout {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px;
  margin-top: auto;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.04);
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.user-menu__logout[data-focused='true'] {
  background: #107c10;
  box-shadow: var(--shadow-xbox-focus);
}

.user-menu__logout-icon {
  width: 18px;
  height: 18px;
  filter: brightness(0) invert(1);
}

.user-menu__logout-label {
  font-size: 16px;
  font-weight: var(--ui-font-weight-bold);
}

.user-menu-transition-enter-active,
.user-menu-transition-leave-active {
  transition: opacity 250ms ease;
}

.user-menu-transition-enter-active .user-menu__panel,
.user-menu-transition-leave-active .user-menu__panel {
  transition: transform 300ms cubic-bezier(0.2, 0, 0, 1);
}

.user-menu-transition-enter-from .user-menu__panel {
  transform: translateX(calc(100% + 24px));
}

.user-menu-transition-leave-to .user-menu__panel {
  transform: translateX(calc(100% + 24px));
}

.user-menu-transition-enter-from,
.user-menu-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .user-menu-anchor {
  left: 0;
}

:global(html[data-ui-density='narrow']) .user-menu__panel {
  width: 100vw;
}
</style>
