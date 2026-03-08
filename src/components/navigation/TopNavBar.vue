<script setup lang="ts">
import type { NodeDef } from '@spatial-navigation/runtime'
import type { TopNavNodeKey } from '../../navigation/spatial-nav.constants'
import { computed } from 'vue'
import {
  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_PRIMARY_TAB_ORDER,
  SPATIAL_NAV_TAB_LEVELS,

} from '../../navigation/spatial-nav.constants'
import SpatialNavIconButton from './SpatialNavIconButton.vue'

type TopNavIcons = Partial<Record<TopNavNodeKey, string>>

interface TopNavBarProps {
  scopeId: string
  downNeighborId?: string
  profileDownNeighborId?: string
  icons?: TopNavIcons
  activeNav?: 'xhome' | 'xcloud' | 'setting'
  profileImageUrl?: string
}

const props = withDefaults(defineProps<TopNavBarProps>(), {
  icons: () => ({}),
  activeNav: 'xhome',
  profileImageUrl: '',
})

const emit = defineEmits<{
  (event: 'select', node: TopNavNodeKey): void
}>()

function buildNeighbors(options: {
  left?: string
  right?: string
  down?: string
}): NodeDef['neighbors'] {
  const neighbors: NodeDef['neighbors'] = {}
  if (options.left !== undefined) {
    neighbors.left = options.left
  }
  if (options.right !== undefined) {
    neighbors.right = options.right
  }
  if (options.down !== undefined) {
    neighbors.down = options.down
  }
  return neighbors
}

// 顶部导航采用显式邻接，确保键盘/手柄移动稳定
const navNeighbors = computed(() => ({
  brand: buildNeighbors({
    right: SPATIAL_NAV_NODE_IDS.topNav.xhome,
    down: props.downNeighborId,
  }),
  xhome: buildNeighbors({
    left: SPATIAL_NAV_NODE_IDS.topNav.brand,
    right: SPATIAL_NAV_NODE_IDS.topNav.xcloud,
    down: props.downNeighborId,
  }),
  xcloud: buildNeighbors({
    left: SPATIAL_NAV_NODE_IDS.topNav.xhome,
    right: SPATIAL_NAV_NODE_IDS.topNav.setting,
    down: props.downNeighborId,
  }),
  setting: buildNeighbors({
    left: SPATIAL_NAV_NODE_IDS.topNav.xcloud,
    right: SPATIAL_NAV_NODE_IDS.topNav.controller,
    down: props.downNeighborId,
  }),
  controller: buildNeighbors({
    left: SPATIAL_NAV_NODE_IDS.topNav.setting,
    right: SPATIAL_NAV_NODE_IDS.topNav.profile,
    down: props.downNeighborId,
  }),
  profile: buildNeighbors({
    left: SPATIAL_NAV_NODE_IDS.topNav.controller,
    down: props.profileDownNeighborId ?? props.downNeighborId,
  }),
}))

function getIcon(node: TopNavNodeKey): string {
  return props.icons[node] ?? ''
}

function emitSelect(node: TopNavNodeKey): void {
  emit('select', node)
}

// 顶部主导航挂到 primary tab 级别，供 LB/RB 切换
function getPrimaryTabIndex(node: TopNavNodeKey): number | undefined {
  return SPATIAL_NAV_PRIMARY_TAB_ORDER[node as keyof typeof SPATIAL_NAV_PRIMARY_TAB_ORDER]
}
</script>

<template>
  <header class="top-nav">
    <div class="top-nav__group top-nav__group--left">
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.brand"
        :scope-id="props.scopeId"
        label="Xbox Logo"
        :neighbors="navNeighbors.brand"
        :icon-src="getIcon('brand')"
        :round="true"
        :on-click="() => emitSelect('brand')"
        :on-confirm="() => emitSelect('brand')"
      />
    </div>

    <nav class="top-nav__group top-nav__group--center" aria-label="Top Navigation">
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.xhome"
        :scope-id="props.scopeId"
        label="XHome"
        :neighbors="navNeighbors.xhome"
        :tab-level="SPATIAL_NAV_TAB_LEVELS.primary"
        :index="{ order: getPrimaryTabIndex('xhome') }"
        :icon-src="getIcon('xhome')"
        :round="true"
        :active="props.activeNav === 'xhome'"
        :on-click="() => emitSelect('xhome')"
        :on-confirm="() => emitSelect('xhome')"
      />
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.xcloud"
        :scope-id="props.scopeId"
        label="XCloud"
        :neighbors="navNeighbors.xcloud"
        :tab-level="SPATIAL_NAV_TAB_LEVELS.primary"
        :index="{ order: getPrimaryTabIndex('xcloud') }"
        :icon-src="getIcon('xcloud')"
        :round="true"
        :active="props.activeNav === 'xcloud'"
        :on-click="() => emitSelect('xcloud')"
        :on-confirm="() => emitSelect('xcloud')"
      />
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.setting"
        :scope-id="props.scopeId"
        label="Setting"
        :neighbors="navNeighbors.setting"
        :tab-level="SPATIAL_NAV_TAB_LEVELS.primary"
        :index="{ order: getPrimaryTabIndex('setting') }"
        :icon-src="getIcon('setting')"
        :round="true"
        :active="props.activeNav === 'setting'"
        :on-click="() => emitSelect('setting')"
        :on-confirm="() => emitSelect('setting')"
      />
    </nav>

    <div class="top-nav__group top-nav__group--right">
      <SpatialNavIconButton
        :id="SPATIAL_NAV_NODE_IDS.topNav.controller"
        :scope-id="props.scopeId"
        label="Controller Status"
        :neighbors="navNeighbors.controller"
        :icon-src="getIcon('controller')"
        :round="true"
        :on-click="() => emitSelect('controller')"
        :on-confirm="() => emitSelect('controller')"
      />

      <div class="top-nav__avatar">
        <SpatialNavIconButton
          :id="SPATIAL_NAV_NODE_IDS.topNav.profile"
          :scope-id="props.scopeId"
          label="Profile"
          :neighbors="navNeighbors.profile"
          :round="true"
          :on-click="() => emitSelect('profile')"
          :on-confirm="() => emitSelect('profile')"
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

.top-nav :deep(.sn-icon-button) {
  width: var(--ui-size-control-xl);
  height: var(--ui-size-control-xl);
}

.top-nav :deep(.sn-icon-button--active) {
  background: color-mix(in srgb, var(--color-state-selected) 58%, transparent);
  backdrop-filter: blur(12px) saturate(118%);
  -webkit-backdrop-filter: blur(12px) saturate(118%);
  box-shadow:
    inset 0 0 0 1px var(--color-border-subtle),
    0 6px 16px rgba(0, 0, 0, 0.12);
}

.top-nav :deep(.sn-icon-button--active[data-focused='true']) {
  background: color-mix(in srgb, var(--color-state-selected) 34%, transparent);
  box-shadow: 0 0 0 var(--focus-ring-width) var(--color-focus-ring-outer) inset;
}

.top-nav :deep(.sn-icon-button__icon-shell),
.top-nav :deep(.sn-icon-button__icon),
.top-nav :deep(.sn-icon-button__icon-empty) {
  width: var(--ui-size-icon-lg);
  height: var(--ui-size-icon-lg);
}

.top-nav__avatar-image,
.top-nav__avatar-placeholder {
  width: var(--ui-size-icon-lg);
  height: var(--ui-size-icon-lg);
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
