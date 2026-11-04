<script setup lang="ts">
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine } from '@/navigation/core'
import { useShellPageHints } from '../components/layout/shell-context'
import { SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import ControllerConnectionSection from './controller/ControllerConnectionSection.vue'
import SettingInputToolsSection from './settings/SettingInputToolsSection.vue'
import SettingSidebar from './settings/SettingSidebar.vue'

const { t } = useI18n()
type ControllerTab = 'connection' | 'settings'
const activeTab = ref<ControllerTab>('connection')
const tabs = computed(() => (['connection', 'settings'] as const).map(key => ({ key, label: t(`controllerPage.${key}`), nodeId: `controller.tabs.${key}` })))
const activeLabel = computed(() => t(`controllerPage.${activeTab.value}`))
const contentRef = ref<HTMLElement | null>(null)
const scrollPositions = { connection: 0, settings: 0 }
let active = true
let disposeTabSwitch: (() => void) | undefined
useShellPageHints(() => ({ status: activeLabel.value, regionLabel: t('shell.hints.categories') }))

function changeTab(key: ControllerTab): void {
  if (key === activeTab.value)
    return
  scrollPositions[activeTab.value] = contentRef.value?.scrollTop ?? 0
  activeTab.value = key
}
function restoreScroll(): void {
  if (contentRef.value)
    contentRef.value.scrollTop = scrollPositions[activeTab.value]
}
onMounted(() => {
  disposeTabSwitch = navigationEngine.onTabSwitch((direction) => {
    if (!active || !navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell))
      return
    const key: ControllerTab = direction === 'next' ? 'settings' : 'connection'
    changeTab(key)
    void nextTick(() => navigationEngine.focusElement(document.getElementById(`controller.tabs.${key}`), false))
  })
})
onActivated(() => {
  active = true
})
onDeactivated(() => {
  active = false
})
onBeforeUnmount(() => disposeTabSwitch?.())
</script>

<template>
  <section class="controller-page ui-page-shell" :aria-label="t('shell.controller')">
    <SettingSidebar :title="t('shell.controller')" :tabs="tabs" :active-tab-key="activeTab" :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell" @tab-change="changeTab" />
    <section class="controller-page__panel" :aria-label="activeLabel">
      <header class="controller-page__header">
        <h1>{{ activeLabel }}</h1>
        <p>{{ t(`controllerPage.${activeTab}Hint`) }}</p>
      </header>
      <div ref="contentRef" class="controller-page__content" data-nav-scroll-container>
        <Transition name="panel-slide" mode="out-in" @after-enter="restoreScroll">
          <!-- 设置编辑器包含多个弹层根节点，单根容器承接子页进出动画。 -->
          <div :key="activeTab">
            <ControllerConnectionSection v-if="activeTab === 'connection'" />
            <SettingInputToolsSection v-else scope-id="controller.content" nav-node-base-id="controller.settings" />
          </div>
        </Transition>
      </div>
    </section>
  </section>
</template>

<style scoped>
.controller-page { display: grid; grid-template-columns: var(--ui-settings-layout-columns); grid-template-rows: var(--ui-settings-layout-rows); gap: var(--space-16); height: 100%; min-height: 0; overflow: hidden; }
.controller-page__panel { display: grid; grid-template-rows: auto minmax(0, 1fr); min-width: 0; min-height: 0; overflow: hidden; }
.controller-page__header { padding: var(--ui-space-lg) var(--ui-settings-side-padding); }
.controller-page__header h1 { margin: 0; font-size: var(--ui-text-title-lg); font-weight: var(--ui-font-weight-semibold); line-height: var(--ui-line-height-tight); }
.controller-page__header p { margin: var(--ui-space-sm) 0 0; color: var(--color-text-secondary); font-size: var(--ui-settings-row-description-size); line-height: var(--ui-line-height-relaxed); }
.controller-page__content { min-height: 0; min-width: 0; overflow: auto; scroll-padding: var(--ui-settings-focus-safe) var(--ui-settings-focus-safe) var(--ui-settings-focus-tail); }
</style>
