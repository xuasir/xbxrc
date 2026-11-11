<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import ExportDiagnosticsButton from '@/components/common/ExportDiagnosticsButton.vue'
import RemoteImage from '@/components/common/RemoteImage.vue'
import SettingInputControl from '@/components/settings/SettingInputControl.vue'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import { mergePersonalAccount } from './library-merge'
import { formatDate, useLibraryFocusFallback } from './library-view'
import LibraryRefresh from './LibraryRefresh.vue'
import LibrarySelectControl from './LibrarySelectControl.vue'
import { useLibraryQuery } from './use-library-query'

const route = useRoute()
const router = useRouter()
const query = useLibraryQuery(() => 'account:self', () => rpc.data.getPersonalAccount({}), mergePersonalAccount)
const state = query.state
const profile = computed(() => state.value.value?.profile)
const search = ref('')
const filter = ref('all')
const friends = computed(() => (state.value.value?.friends ?? []).filter(p => `${p.gamertag} ${p.displayName}`.toLowerCase().includes(search.value.toLowerCase()) && (filter.value === 'all' || (p.presenceState === 'Online' && (filter.value === 'online' || !!p.currentTitleId)))).sort((a, b) => Number(b.presenceState === 'Online') - Number(a.presenceState === 'Online') || Number(!!b.currentTitleId) - Number(!!a.currentTitleId) || a.gamertag.localeCompare(b.gamertag)))
useLibraryFocusFallback(() => ['my.friends.filter', ...friends.value.map(p => `my.friend.${p.xuid}`)])
</script>

<template>
  <section class="library-stack" aria-label="账户与好友" :aria-busy="state.loading">
    <header class="library-toolbar">
      <h1 class="library-section-title">
        账户与好友
      </h1><span class="library-caption">{{ state.updatedAt ? `更新于 ${formatDate(state.updatedAt)}` : '' }}</span><LibraryRefresh id="my.account.refresh" :busy="state.loading" @refresh="query.refresh" />
    </header>
    <p v-if="state.error" class="library-notice" role="alert">
      {{ state.error }}
      <ExportDiagnosticsButton id="account.error.export" scope-id="app.shell" :issue-id="state.issueId" />
    </p>
    <p v-for="notice in state.value?.notices" :key="notice" class="library-notice" role="status">
      {{ notice }}
    </p>
    <div v-if="profile" class="library-profile">
      <RemoteImage class="library-profile__avatar" :src="profile.avatarUrl" :fallback="(profile.displayName || profile.gamertag).slice(0, 1)" />
      <div>
        <h2>{{ profile.displayName || profile.gamertag }}</h2><p class="library-copy">
          {{ profile.gamertag }} · {{ profile.gamerscore ?? '—' }} G
        </p><p class="library-copy">
          {{ profile.presenceState === 'Online' ? '在线' : profile.presenceState === 'Offline' ? '离线' : '在线状态暂缺' }}<span v-if="profile.presenceText"> · {{ profile.presenceText }}</span>
        </p>
      </div>
    </div>
    <div class="library-toolbar">
      <SettingInputControl id="my.friends.search" v-model="search" scope-id="app.shell" label="搜索好友" placeholder="搜索好友" />
      <LibrarySelectControl id="my.friends.filter" v-model="filter" label="好友状态" :options="[{ value: 'all', label: '全部' }, { value: 'online', label: '在线' }, { value: 'playing', label: '游戏中' }]" />
      <span class="library-caption">{{ state.value?.friends ? `已加载 ${state.value.friends.length} 位好友` : state.loading ? '正在查询好友…' : '好友数据暂缺' }}</span>
    </div>
    <div class="library-list">
      <Focusable v-for="person in friends" :id="`my.friend.${person.xuid}`" :key="person.xuid" as="button" type="button" class="library-row library-row--person" @click="router.push({ name: 'my-friend', params: { targetXuid: person.xuid }, query: { from: route.fullPath } })">
        <RemoteImage class="library-thumb" :src="person.avatarUrl" :fallback="(person.gamertag || person.displayName).slice(0, 1)" />
        <span class="library-row__body"><strong>{{ person.gamertag || person.displayName }}</strong><span class="library-caption">{{ person.presenceState === 'Online' ? '在线' : person.presenceState === 'Offline' ? '离线' : '状态暂缺' }}<span v-if="person.presenceText"> · {{ person.presenceText }}</span></span></span><span class="library-row__value">{{ person.gamerscore ?? '—' }} G</span>
      </Focusable>
    </div>
    <p v-if="state.loading && !state.value" class="library-state" role="status">
      正在加载账户与好友…
    </p>
    <p v-else-if="state.value?.friends && !friends.length" class="library-state">
      {{ search || filter !== 'all' ? '当前筛选结果为空' : 'Xbox 好友列表为空' }}
    </p>
  </section>
</template>
