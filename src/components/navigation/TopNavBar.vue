<script setup lang="ts">
import type { TopNavNodeKey } from '../../navigation/spatial-nav.constants'
import {
  SPATIAL_NAV_NODE_IDS,
} from '../../navigation/spatial-nav.constants'
import SpatialNavIconButton from './SpatialNavIconButton.vue'

type TopNavIcons = Partial<Record<TopNavNodeKey, string>>

interface TopNavBarProps {
  icons?: TopNavIcons
  activeNav?: 'xhome' | 'xcloud' | 'setting'
  profileImageUrl?: string
  controllerActive?: boolean
  showController?: boolean
}

const props = withDefaults(defineProps<TopNavBarProps>(), {
  icons: () => ({}),
  activeNav: 'xhome',
  profileImageUrl: '',
  controllerActive: false,
  showController: true,
})

const emit = defineEmits<{
  (event: 'select', node: TopNavNodeKey): void
}>()

function getIcon(node: TopNavNodeKey): string {
  return props.icons[node] ?? ''
}

function emitSelect(node: TopNavNodeKey): void {
  emit('select', node)
}
</script>

<template>
  <header class="top-nav">
    <div class="top-nav__group top-nav__group--left">
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.brand"
        label="Xbox Logo"
        :icon-src="getIcon('brand')"
        :round="true"
        @click="() => emitSelect('brand')"
      />
    </div>

    <nav class="top-nav__group top-nav__group--center" aria-label="Top Navigation">
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.xhome"
        label="XHome"
        :icon-src="getIcon('xhome')"
        :round="true"
        :active="props.activeNav === 'xhome'"
        @click="() => emitSelect('xhome')"
      />
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.xcloud"
        label="XCloud"
        :icon-src="getIcon('xcloud')"
        :round="true"
        :active="props.activeNav === 'xcloud'"
        @click="() => emitSelect('xcloud')"
      />
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.setting"
        label="Setting"
        :icon-src="getIcon('setting')"
        :round="true"
        :active="props.activeNav === 'setting'"
        @click="() => emitSelect('setting')"
      />
    </nav>

    <div class="top-nav__group top-nav__group--right">
      <SpatialNavIconButton
        v-if="props.showController"
        :id="SPATIAL_NAV_NODE_IDS.topNav.controller"
        label="Controller Status"
        :icon-src="getIcon('controller')"
        :round="true"
        :active="props.controllerActive"
        @click="() => emitSelect('controller')"
      />

      <div class="top-nav__avatar">
        <SpatialNavIconButton
          :id="SPATIAL_NAV_NODE_IDS.topNav.profile"
          label="Profile"
          :round="true"
          @click="() => emitSelect('profile')"
        >
          <template #default>
            <img
              v-if="props.profileImageUrl"
              class="top-nav__avatar-image"
              :src="props.profileImageUrl"
              alt="Profile avatar"
            >
            <span v-else class="top-nav__avatar-placeholder" aria-hidden="true" />
          </template>
        </SpatialNavIconButton>
      </div>
    </div>
  </header>
</template>

<style scoped>
.top-nav {
  position: relative;
  z-index: 1;
  flex: 0 0 auto;
  height: calc(var(--ui-size-nav-height) + var(--ui-space-2));
  padding: var(--ui-space-xl) var(--ui-page-inset);
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
  align-items: center;
  gap: var(--ui-space-xl);
}

.top-nav__group {
  display: flex;
  align-items: center;
}

.top-nav__group--left {
  justify-self: start;
  justify-content: flex-start;
}

.top-nav__group--center {
  justify-self: center;
  gap: var(--ui-space-md);
  justify-content: center;
}

.top-nav__group--right {
  justify-self: end;
  gap: var(--ui-space-md);
  justify-content: flex-end;
}

.top-nav__avatar {
  display: inline-flex;
}

.top-nav__avatar :deep(.sn-icon-button) {
  padding: 0;
  overflow: hidden;
}

.top-nav :deep(.sn-icon-button) {
  width: var(--ui-size-control-xl);
  height: var(--ui-size-control-xl);
}

.top-nav :deep(.sn-icon-button--active) {
  background: var(--color-state-selected);
  box-shadow: inset 0 0 0 1px var(--color-border-subtle);
}

.top-nav :deep(.sn-icon-button--active[data-focused='true']) {
  background: var(--color-state-selected);
  box-shadow: var(--shadow-xbox-focus);
}

.top-nav :deep(.sn-icon-button__icon-shell),
.top-nav :deep(.sn-icon-button__icon),
.top-nav :deep(.sn-icon-button__icon-empty) {
  width: var(--ui-size-icon-lg);
  height: var(--ui-size-icon-lg);
}

.top-nav__avatar-image,
.top-nav__avatar-placeholder {
  width: 100%;
  height: 100%;
  border-radius: 999px;
  display: block;
}

.top-nav__avatar-image {
  object-fit: cover;
}

.top-nav__avatar-placeholder {
  border: 1px dashed var(--color-border-subtle);
  background: var(--color-surface-2);
}

:global(html[data-ui-density='compact']) .top-nav,
:global(html[data-ui-density='narrow']) .top-nav {
  grid-template-columns: auto auto auto;
  padding: var(--ui-space-3) calc(var(--ui-space-3) + var(--ui-space-1));
  gap: calc(var(--ui-space-1) + 2px);
}

:global(html[data-ui-density='compact']) .top-nav__group--center,
:global(html[data-ui-density='compact']) .top-nav__group--right,
:global(html[data-ui-density='narrow']) .top-nav__group--center,
:global(html[data-ui-density='narrow']) .top-nav__group--right {
  gap: var(--ui-space-2);
}

:global(html[data-ui-density='narrow']) .top-nav {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  grid-template-areas:
    'left right'
    'center center';
  height: auto;
}

:global(html[data-ui-density='narrow']) .top-nav__group--left {
  grid-area: left;
}

:global(html[data-ui-density='narrow']) .top-nav__group--center {
  grid-area: center;
  width: 100%;
  justify-content: flex-start;
}

:global(html[data-ui-density='narrow']) .top-nav__group--right {
  grid-area: right;
  margin-left: 0;
}
</style>
