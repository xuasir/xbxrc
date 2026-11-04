<script setup lang="ts">
import type { AppPageRouteName, TopNavNodeKey } from '../../navigation/spatial-nav.constants'
import { useI18n } from 'vue-i18n'
import { APP_PAGE_LABEL_KEYS, SPATIAL_NAV_NODE_IDS } from '../../navigation/spatial-nav.constants'
import SpatialNavIconButton from './SpatialNavIconButton.vue'

type TopNavIcons = Partial<Record<TopNavNodeKey, string>>
const props = withDefaults(defineProps<{
  icons?: TopNavIcons
  activeNav?: AppPageRouteName
  profileImageUrl?: string
  profileDisplayName?: string
  profileOpen?: boolean
  controllerActive?: boolean
  showController?: boolean
}>(), { icons: () => ({}), activeNav: 'xhome', profileImageUrl: '', profileDisplayName: 'X', profileOpen: false, controllerActive: false, showController: true })
const emit = defineEmits<{ select: [node: TopNavNodeKey] }>()
const { t } = useI18n()
const pages = [{ key: 'xhome', label: 'shell.home' }, { key: 'xcloud', label: 'shell.cloud' }] as const
</script>

<template>
  <div class="top-nav">
    <header class="top-nav__header" data-tauri-drag-region>
      <SpatialNavIconButton :id="SPATIAL_NAV_NODE_IDS.topNav.brand" class="top-nav__brand" :label="t('shell.brand')" @click="emit('select', 'brand')">
        <img :src="props.icons.brand" alt="" class="top-nav__brand-image"><span>XBXRC</span>
      </SpatialNavIconButton>
      <span class="top-nav__context" data-tauri-drag-region>{{ t(APP_PAGE_LABEL_KEYS[props.activeNav]) }}</span>
      <SpatialNavIconButton :id="SPATIAL_NAV_NODE_IDS.topNav.profile" class="top-nav__avatar" :label="t('shell.profile')" :round="true" :aria-expanded="props.profileOpen" aria-controls="user-menu-panel" @click="emit('select', 'profile')">
        <img v-if="props.profileImageUrl" class="top-nav__avatar-image" :src="props.profileImageUrl" alt="">
        <span v-else class="top-nav__avatar-placeholder" aria-hidden="true">{{ props.profileDisplayName.slice(0, 1).toUpperCase() }}</span>
      </SpatialNavIconButton>
    </header>
    <nav class="top-nav__dock" :aria-label="t('shell.navigation')">
      <div class="top-nav__group">
        <SpatialNavIconButton v-for="page in pages" :id="SPATIAL_NAV_NODE_IDS.topNav[page.key]" :key="page.key" :label="t(page.label)" :active="props.activeNav === page.key" :aria-current="props.activeNav === page.key ? 'page' : undefined" @click="emit('select', page.key)">
          <img class="top-nav__icon" :src="props.icons[page.key]" alt=""><span>{{ t(page.label) }}</span>
        </SpatialNavIconButton>
      </div>
      <div class="top-nav__group">
        <SpatialNavIconButton :id="SPATIAL_NAV_NODE_IDS.topNav.setting" :label="t('shell.settings')" :active="props.activeNav === 'setting'" :aria-current="props.activeNav === 'setting' ? 'page' : undefined" @click="emit('select', 'setting')">
          <img class="top-nav__icon" :src="props.icons.setting" alt=""><span>{{ t('shell.settings') }}</span>
        </SpatialNavIconButton>
        <SpatialNavIconButton v-if="props.showController" :id="SPATIAL_NAV_NODE_IDS.topNav.controller" :label="t('shell.controller')" :active="props.activeNav === 'controller'" :aria-current="props.activeNav === 'controller' ? 'page' : undefined" @click="emit('select', 'controller')">
          <img class="top-nav__icon" :src="props.icons.controller" alt=""><span>{{ t('shell.controller') }}</span><span v-if="props.controllerActive" class="top-nav__connected" aria-hidden="true" />
        </SpatialNavIconButton>
      </div>
    </nav>
  </div>
</template>

<style scoped>
.top-nav { display: contents; }
.top-nav__header { grid-column: 1 / -1; grid-row: 1; position: relative; z-index: var(--z-nav); display: flex; align-items: center; gap: var(--space-24); min-width: 0; height: var(--ui-shell-header-height); padding-left: var(--space-12); background: transparent; }
.top-nav__brand { width: auto; height: 36px; gap: var(--space-8); padding: 0; font-size: 14px; font-weight: 600; letter-spacing: .03em; }
.top-nav__brand-image { width: 24px; height: 24px; filter: var(--ui-nav-icon-filter); }
.top-nav__context { padding-left: var(--space-24); border-left: 1px solid var(--ui-border-subtle); color: var(--color-text-secondary); font-size: 12px; line-height: 18px; }
.top-nav__avatar { width: 36px; height: 36px; flex-shrink: 0; margin-left: auto; padding: 0; overflow: visible; border: 1px solid var(--ui-border-subtle); background: var(--ui-material-glass); box-shadow: inset 0 1px 0 var(--ui-material-edge), var(--ui-shadow-low); }
.top-nav__avatar-image, .top-nav__avatar-placeholder { width: 100%; height: 100%; border-radius: 50%; }
.top-nav__avatar-image { object-fit: cover; }
.top-nav__avatar-placeholder { display: flex; align-items: center; justify-content: center; color: var(--ui-icon-text); font-size: 16px; }
.top-nav__dock { grid-column: 1; grid-row: 2; align-self: start; position: relative; z-index: var(--z-nav); display: flex; flex-direction: column; gap: var(--space-24); padding-top: var(--space-24); }
.top-nav__group { display: flex; flex-direction: column; gap: var(--space-8); padding: var(--space-8) 7px; background: transparent; }
.top-nav__dock :deep(.sn-icon-button) { position: relative; display: flex; flex-direction: column; gap: var(--space-4); width: 56px; min-height: 56px; height: auto; padding: var(--space-4); font-size: 12px; line-height: 16px; color: var(--ui-icon-text); }
.top-nav__icon { width: 20px; height: 20px; object-fit: contain; filter: var(--ui-nav-icon-filter); }
.top-nav__connected { position: absolute; right: 12px; top: 8px; width: 5px; height: 5px; border-radius: 50%; background: var(--ui-status-positive); }
.top-nav :deep(.sn-icon-button:hover), .top-nav :deep(.sn-icon-button[data-focused='true']) { background: var(--ui-material-hover); color: var(--color-text-primary); border-color: transparent; }
.top-nav :deep(.sn-icon-button--active) { background: var(--ui-material-selected); border-color: transparent; box-shadow: none; color: var(--color-text-primary); }
.top-nav :deep(.sn-icon-button[data-focused='true']) { box-shadow: var(--shadow-xbox-focus); }
.top-nav :deep(.top-nav__brand), .top-nav :deep(.top-nav__brand:hover), .top-nav :deep(.top-nav__brand[data-focused='true']) { background: transparent; }
@media(max-width:700px) { .top-nav__header { grid-column: 1; padding-left: var(--space-4); gap: var(--space-12); } .top-nav__context { padding-left: var(--space-12); } .top-nav__dock { grid-column: 1; grid-row: 2; flex-direction: row; gap: var(--space-12); padding-top: 0; align-self: stretch; } .top-nav__group { flex: 1; flex-direction: row; padding: var(--space-4); gap: var(--space-4); } .top-nav__dock :deep(.sn-icon-button) { flex: 1; min-width: 44px; width: auto; min-height: 48px; } }
</style>
