<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useShellPageHints } from '@/components/layout/shell-context'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import GameHistoryPanel from './library/GameHistoryPanel.vue'
import { mergePersonalAccount } from './library/library-merge'
import { MY_LABELS, MY_SECTIONS, useLibraryNavigation, useLibraryRoute } from './library/library-view'
import LibraryBack from './library/LibraryBack.vue'
import LibraryRefresh from './library/LibraryRefresh.vue'
import { useLibraryQuery } from './library/use-library-query'
import SettingSidebar from './settings/SettingSidebar.vue'
import './library/library.scss'

const route = useLibraryRoute()
const router = useRouter()
const target = computed(() => String(route.params.targetXuid || ''))
const query = useLibraryQuery(() => `account:${target.value}`, () => rpc.data.getPersonalAccount({ targetXuid: target.value }), mergePersonalAccount)
const profile = computed(() => query.state.value.value?.profile)
const ownAccount = useLibraryQuery(() => 'account:self', () => rpc.data.getPersonalAccount({}), mergePersonalAccount)
const presence = computed(() => ownAccount.state.value.value?.friends?.find(p => p.xuid === target.value))
const panel = ref<HTMLElement | null>(null)
const tabs = MY_SECTIONS.map(key => ({ key, label: MY_LABELS[key], nodeId: `library.tab.${key}` }))
function select(key: string) {
  void router.push(`/my/${key}`)
}
const nav = useLibraryNavigation(() => panel.value, () => route.fullPath, () => MY_SECTIONS, () => 'account', select, () => '/my/account')
useShellPageHints(() => ({ regionLabel: '分类', status: query.state.value.error || '账户与好友', error: !!query.state.value.error }))
async function refresh() {
  await Promise.allSettled([query.refresh(), ownAccount.refresh()])
}
</script>

<template>
  <section class="library-page ui-page-shell" @nav-back="nav.navBack">
    <div class="library-layout">
      <SettingSidebar title="我的" :tabs="tabs" active-tab-key="account" scope-id="app.shell" @tab-change="select" /><section class="library-panel">
        <header class="library-header">
          <LibraryBack id="friend.back" label="返回好友列表" @back="nav.back" /><LibraryRefresh id="friend.refresh" :busy="query.state.value.loading || ownAccount.state.value.loading" @refresh="refresh" />
        </header><div ref="panel" class="library-content library-stack" data-nav-scroll-container>
          <p v-if="query.state.value.error" class="library-notice" role="alert">
            {{ query.state.value.error }}
          </p><p v-for="notice in query.state.value.value?.notices" :key="notice" class="library-notice" role="status">
            {{ notice }}
          </p><div v-if="profile || presence" class="library-profile">
            <img v-if="profile?.avatarUrl || presence?.avatarUrl" :src="profile?.avatarUrl || presence?.avatarUrl || ''" alt="" class="library-profile__avatar"><div>
              <h2>{{ profile?.gamertag || presence?.gamertag }}</h2><p class="library-copy">
                {{ profile?.gamerscore || presence?.gamerscore || '—' }} G
              </p><p v-if="presence" class="library-copy">
                {{ presence.presenceState === 'Online' ? '在线' : presence.presenceState === 'Offline' ? '离线' : '状态暂缺' }}<span v-if="presence.presenceText"> · {{ presence.presenceText }}</span>
              </p><Focusable v-if="presence?.currentTitleId" id="friend.current-game" as="button" class="library-control library-control--link" @click="router.push({ name: 'my-game', params: { xboxTitleId: presence.currentTitleId }, query: { targetXuid: target, tab: 'achievements', from: route.fullPath } })">
                正在玩的游戏 · 成就对比
              </Focusable>
            </div>
          </div><GameHistoryPanel :key="target" :target-xuid="target" />
        </div>
      </section>
    </div>
  </section>
</template>
