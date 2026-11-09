<script setup lang="ts" generic="TabKey extends string">
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'

defineProps<{
  tabs: { key: TabKey, label: string, nodeId: string }[]
  activeTabKey: TabKey
  scopeId: string
  title?: string
}>()

const emit = defineEmits<{
  tabChange: [tabKey: TabKey]
}>()

const { t } = useI18n()
</script>

<template>
  <aside class="setting-sidebar" :aria-label="title || t('setting.aria.groups')">
    <header class="setting-sidebar__header">
      <h1 class="setting-sidebar__title">
        {{ title || t('setting.title') }}
      </h1>
    </header>

    <nav class="setting-sidebar__nav" data-nav-scroll-container>
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
        :aria-current="activeTabKey === tab.key ? 'page' : undefined"
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
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--ui-settings-sidebar-gap);
  padding: var(--space-4);
  background: transparent;

}
.setting-sidebar__header { display: var(--ui-settings-sidebar-header-display); padding: var(--ui-space-sm) var(--ui-settings-focus-safe); }
.setting-sidebar__title { margin: 0; font-size: var(--ui-text-title-md); font-weight: 500; color: var(--color-text-primary); }
.setting-sidebar__nav {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: var(--ui-settings-sidebar-nav-direction);
  gap: var(--ui-space-sm);
  overflow: auto;
  padding: var(--ui-settings-focus-safe) var(--ui-settings-focus-safe) var(--ui-settings-focus-tail);
  scroll-padding: var(--ui-settings-focus-safe) var(--ui-settings-focus-safe) var(--ui-settings-focus-tail);
}
.setting-sidebar__tab {
  position: relative;
  flex: 0 0 auto;
  min-height: var(--ui-settings-row-min-height);
  width: var(--ui-settings-sidebar-tab-width);
  padding: var(--ui-settings-row-padding);
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: var(--color-text-secondary);
  text-align: left;
  white-space: nowrap;
  scroll-margin: var(--ui-settings-focus-safe);
  transition: border-color var(--ui-motion-fast), background-color var(--ui-motion-fast), box-shadow var(--ui-motion-fast);
}
.setting-sidebar__tab:hover { background: var(--ui-material-hover); }
.setting-sidebar__tab--active { background: var(--ui-choice-selected-bg); color: var(--color-text-primary); }
.setting-sidebar__tab--active::after { content: ''; position: absolute; left: 0; top: 25%; bottom: 25%; width: 3px; border-radius: var(--ui-radius-sm); background: var(--ui-status-positive); }
.setting-sidebar__tab[data-focused='true'] { border-color: transparent; color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 10; }
.setting-sidebar__tab--active:hover { background: var(--ui-choice-selected-bg); }
.setting-sidebar__tab-label { font-size: var(--ui-settings-row-label-size); line-height: var(--ui-line-height-default); font-weight: 400; }
.setting-sidebar__tab[data-focused='true'] .setting-sidebar__tab-label { color: var(--ui-focus-text); }
@media(max-width:600px) {
  .setting-sidebar__nav {
    padding-bottom: var(--ui-settings-focus-safe);
    scroll-padding-bottom: var(--ui-settings-focus-safe);
  }
  .setting-sidebar__tab--active::after {
    left: var(--ui-space-lg);
    right: var(--ui-space-lg);
    top: auto;
    bottom: 0;
    width: auto;
    height: 3px;
  }
}
</style>
