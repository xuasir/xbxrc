<script setup lang="ts">
import type { SettingTabKey } from '../../navigation/spatial-nav.constants'
import type { SettingTabNavItem } from './setting-types'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'

defineProps<{
  tabs: SettingTabNavItem[]
  activeTabKey: SettingTabKey
  scopeId: string
}>()

const emit = defineEmits<{
  tabChange: [tabKey: SettingTabKey]
}>()

const { t } = useI18n()
</script>

<template>
  <aside class="setting-sidebar" :aria-label="t('setting.aria.groups')">
    <header class="setting-sidebar__header">
      <h1 class="setting-sidebar__title">
        {{ t('setting.title') }}
      </h1>
    </header>

    <nav class="setting-sidebar__nav">
      <Focusable
        v-for="tab in tabs"
        :id="tab.nodeId"
        :key="tab.key"
        as="button"
        type="button"
        class="setting-sidebar__tab"
        :class="{ 'setting-sidebar__tab--active': activeTabKey === tab.key }"
        :scope-id="scopeId"
        :aria-label="tab.label"
        @click="emit('tabChange', tab.key)"
      >
        <span class="setting-sidebar__tab-label">{{ tab.label }}</span>
      </Focusable>
    </nav>
  </aside>
</template>

<style scoped>
.setting-sidebar {
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 32px;
  padding: 44px 20px 32px;
  background: var(--ui-page-bg);
  position: relative;
  z-index: 2;
  border-right: 1px solid var(--ui-border-subtle);
}

.setting-sidebar__header {
  padding: 0 16px;
}

.setting-sidebar__title {
  margin: 0;
  font-size: clamp(24px, 3vw, 32px);
  line-height: 1.1;
  font-weight: 900;
  letter-spacing: -0.02em;
  color: var(--color-text-primary);
}

.setting-sidebar__nav {
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  overflow-y: auto;
  overflow-x: visible;
  padding: 12px 16px;
  margin: 0 -4px;
}

.setting-sidebar__tab {
  position: relative;
  display: inline-flex;
  align-items: center;
  width: 100%;
  min-height: 52px;
  padding: 0 20px;
  border: 2px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-secondary);
  text-align: left;
  transition: all var(--ui-motion-fast);
  transform-origin: left center;
}

.setting-sidebar__tab:hover {
  background: var(--color-state-hover);
  color: var(--color-text-primary);
}

.setting-sidebar__tab::before {
  content: '';
  position: absolute;
  left: 0;
  top: 12px;
  bottom: 12px;
  width: 4px;
  background: var(--brand-primary);
  border-radius: 0 2px 2px 0;
  opacity: 0;
  transition: opacity var(--ui-motion-fast);
}

.setting-sidebar__tab--active {
  background: var(--color-state-selected);
  color: var(--ui-page-text);
}

.setting-sidebar__tab--active::before {
  opacity: 1;
}

.setting-sidebar__tab[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  z-index: 10;
}

.setting-sidebar__tab[data-focused='true']::before {
  background: var(--brand-primary);
}

.setting-sidebar__tab-label {
  font-size: 16px;
  line-height: 1.2;
  font-weight: 700;
}

:global(html[data-ui-density='narrow']) .setting-sidebar {
  mask-image: none;
  border-bottom: 1px solid var(--ui-border-subtle);
}
</style>
