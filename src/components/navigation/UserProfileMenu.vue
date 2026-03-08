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
  background: rgba(0, 0, 0, 0.08);
  cursor: default;
}

.user-menu-anchor {
  position: absolute;
  top: calc(var(--ui-space-lg) + var(--ui-size-control-sm) + 10px);
  right: calc(var(--ui-page-inset) + var(--ui-space-1));
  pointer-events: none;
}

.user-menu__panel {
  width: min(calc(100vw - var(--ui-space-5xl)), var(--ui-user-menu-panel-width));
  pointer-events: auto;
  position: relative;
  padding: var(--ui-user-menu-panel-padding);
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-user-menu-panel-radius);
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.06), rgba(255, 255, 255, 0.02)),
    radial-gradient(circle at top right, var(--ui-page-glow-soft), transparent 48%),
    var(--ui-surface-panel-strong);
  backdrop-filter: blur(14px) saturate(110%);
  box-shadow:
    0 18px 32px rgba(0, 0, 0, 0.24),
    0 0 0 1px rgba(255, 255, 255, 0.02);
  color: var(--ui-page-text);
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
  top: var(--ui-space-2);
  right: var(--ui-space-2);
  width: var(--ui-user-menu-close-size);
  height: var(--ui-user-menu-close-size);
  border: 0;
  border-radius: var(--ui-radius-pill);
  background: transparent;
  cursor: pointer;
  opacity: 0.82;
  transition:
    background-color var(--ui-motion-fast),
    opacity var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.user-menu__close[data-focused='true'] {
  background: color-mix(in srgb, var(--ui-focus-surface) 36%, transparent);
  box-shadow: var(--ui-focus-ring-shadow);
  opacity: 1;
}

.user-menu__close:hover {
  background: var(--ui-focus-surface);
  opacity: 1;
}

.user-menu__close-line {
  position: absolute;
  top: 8px;
  left: 4px;
  width: 9px;
  height: var(--ui-stroke-base);
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
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: var(--ui-space-sm);
  align-items: center;
  padding: var(--ui-space-2) var(--ui-space-3) 3px;
}

.user-menu__avatar-wrap {
  position: relative;
  width: var(--ui-user-menu-avatar-size);
  height: var(--ui-user-menu-avatar-size);
}

.user-menu__avatar {
  width: var(--ui-user-menu-avatar-size);
  height: var(--ui-user-menu-avatar-size);
  border-radius: var(--ui-radius-pill);
  object-fit: cover;
}

.user-menu__avatar--placeholder {
  background: var(--ui-surface-placeholder);
  border: 1px solid var(--ui-border-placeholder);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: var(--ui-text-body-xl);
  font-weight: var(--ui-font-weight-semibold);
}

.user-menu__avatar-online {
  position: absolute;
  right: 1px;
  bottom: 1px;
  width: 8px;
  height: 8px;
  border: 1px solid var(--ui-surface-panel-strong);
  border-radius: var(--ui-radius-pill);
  background: var(--ui-status-positive);
}

.user-menu__identity {
  min-width: 0;
}

.user-menu__name-line {
  display: flex;
  align-items: baseline;
  gap: var(--ui-space-2xs);
  flex-wrap: wrap;
}

.user-menu__display-name,
.user-menu__secondary-name,
.user-menu__score,
.user-menu__status-value,
.user-menu__logout-label {
  margin: 0;
}

.user-menu__display-name {
  font-size: var(--ui-user-menu-display-name-size);
  line-height: 1;
  font-weight: var(--ui-font-weight-bold);
  letter-spacing: 0.01em;
}

.user-menu__secondary-name {
  color: var(--ui-page-text-soft);
  font-size: var(--ui-user-menu-secondary-size);
  font-weight: var(--ui-font-weight-semibold);
}

.user-menu__score-row {
  margin-top: var(--ui-space-1);
  display: flex;
  align-items: center;
  gap: var(--ui-space-sm);
}

.user-menu__score-badge {
  width: 16px;
  height: 16px;
  border-radius: var(--ui-radius-pill);
  background: rgba(243, 244, 246, 0.96);
  color: rgb(26, 27, 30);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: var(--ui-font-weight-bold);
}

.user-menu__score {
  font-size: var(--ui-user-menu-secondary-size);
  color: var(--ui-page-text);
}

.user-menu__status {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 8px;
  min-height: var(--ui-user-menu-status-min-height);
  padding: 0 var(--ui-space-3);
}

.user-menu__status-indicator {
  width: 10px;
  height: 10px;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-status-positive);
  flex: 0 0 auto;
}

.user-menu__status-copy {
  min-width: 0;
}

.user-menu__status-value,
.user-menu__logout-label {
  font-size: 10px;
  line-height: var(--ui-line-height-tight);
  font-weight: var(--ui-font-weight-semibold);
  color: var(--ui-page-text);
}

.user-menu__divider {
  height: var(--ui-stroke-thin);
  margin-top: 4px;
  margin-bottom: 7px;
  background: var(--ui-border-subtle);
}

.user-menu__logout {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 6px;
  min-height: var(--ui-user-menu-logout-min-height);
  padding: 9px var(--ui-space-3);
  text-align: left;
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.user-menu__logout[data-focused='true'] {
  border-color: var(--ui-border-focus);
  background: color-mix(in srgb, var(--ui-focus-surface) 36%, transparent);
  box-shadow: var(--ui-focus-ring-shadow);
}

.user-menu__logout:hover {
  background: var(--ui-focus-surface);
  box-shadow: none;
}

.user-menu__logout-icon {
  width: 11px;
  height: 11px;
  object-fit: contain;
  display: block;
  flex: 0 0 auto;
  filter: var(--ui-nav-icon-filter);
}

.user-menu-transition-enter-active,
.user-menu-transition-leave-active {
  transition: opacity 180ms ease;
}

/* 开关时让遮罩先淡入，面板轻微上浮，避免弹窗切换过硬。 */
.user-menu-transition-enter-active .user-menu-layer__backdrop,
.user-menu-transition-leave-active .user-menu-layer__backdrop {
  transition: opacity 180ms ease;
}

.user-menu-transition-enter-active .user-menu__panel,
.user-menu-transition-leave-active .user-menu__panel {
  transition:
    opacity 180ms ease,
    transform 220ms cubic-bezier(0.22, 1, 0.36, 1),
    filter 220ms ease;
}

.user-menu-transition-enter-from,
.user-menu-transition-leave-to {
  opacity: 0;
}

.user-menu-transition-enter-from .user-menu-layer__backdrop,
.user-menu-transition-leave-to .user-menu-layer__backdrop {
  opacity: 0;
}

.user-menu-transition-enter-from .user-menu__panel,
.user-menu-transition-leave-to .user-menu__panel {
  opacity: 0;
  transform: translateY(-8px) scale(0.96);
  filter: blur(4px);
}

:global(html[data-ui-density='narrow']) .user-menu-anchor {
  left: var(--ui-space-3);
  right: var(--ui-space-3);
  top: calc(var(--ui-size-nav-height) + var(--ui-space-4xl));
}

:global(html[data-ui-density='compact']) .user-menu__block--info,
:global(html[data-ui-density='narrow']) .user-menu__block--info {
  grid-template-columns: 1fr;
  gap: var(--ui-space-sm);
  padding: 3px var(--ui-space-1) 0;
}
</style>
