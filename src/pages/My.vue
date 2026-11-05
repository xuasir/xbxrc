<script setup lang="ts">
import type { MySection } from './library/library-view'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useShellPageHints } from '@/components/layout/shell-context'
import { SPATIAL_NAV_SCOPE_IDS } from '@/navigation/spatial-nav.constants'
import AccountPanel from './library/AccountPanel.vue'
import AlbumPanel from './library/AlbumPanel.vue'
import GameHistoryPanel from './library/GameHistoryPanel.vue'
import { MY_LABELS, MY_SECTIONS, useLibraryNavigation, useLibraryRoute } from './library/library-view'
import SettingSidebar from './settings/SettingSidebar.vue'
import './library/library.scss'

const route = useLibraryRoute()
const router = useRouter()
const panel = ref<HTMLElement | null>(null)
const active = computed(() => (route.params.section || 'account') as MySection)
const tabs = MY_SECTIONS.map(key => ({ key, label: MY_LABELS[key], nodeId: `library.tab.${key}` }))
const component = computed(() => active.value === 'account' ? AccountPanel : active.value === 'album' ? AlbumPanel : GameHistoryPanel)
function select(key: string) {
  void router.push(`/my/${key}`)
}
useShellPageHints(() => ({ regionLabel: '分类', status: MY_LABELS[active.value] }))
useLibraryNavigation(() => panel.value, () => `/my/${active.value}`, () => MY_SECTIONS, () => active.value, select, () => '/xhome')
</script>

<template>
  <section class="library-page ui-page-shell">
    <div class="library-layout">
      <SettingSidebar title="我的" :tabs="tabs" :active-tab-key="active" :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell" @tab-change="select" />
      <div ref="panel" class="library-content" data-nav-scroll-container>
        <KeepAlive><component :is="component" :key="active" :mode="active" /></KeepAlive>
      </div>
    </div>
  </section>
</template>
