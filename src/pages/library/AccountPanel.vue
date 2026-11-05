<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import SettingInputControl from '@/components/settings/SettingInputControl.vue'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import { mergePersonalAccount } from './library-merge'
import { formatDate, useLibraryFocusFallback } from './library-view'
import LibraryRefresh from './LibraryRefresh.vue'
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
    </p>
    <p v-for="notice in state.value?.notices" :key="notice" class="library-notice" role="status">
      {{ notice }}
    </p>
    <div v-if="profile" class="library-profile">
      <img v-if="profile.avatarUrl" class="library-profile__avatar" :src="profile.avatarUrl" alt="">
      <span v-else class="library-profile__avatar" aria-hidden="true">{{ (profile.displayName || profile.gamertag).slice(0, 1) }}</span>
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
      <Focusable id="my.friends.filter" :value="filter" as="select" class="library-control" aria-label="好友状态" @change="filter = ($event.target as HTMLSelectElement).value">
        <option value="all">
          全部
        </option><option value="online">
          在线
        </option><option value="playing">
          游戏中
        </option>
      </Focusable>
      <span class="library-caption">已加载 {{ state.value?.friends?.length ?? 0 }} 位好友</span>
    </div>
    <div class="library-list">
      <Focusable v-for="person in friends" :id="`my.friend.${person.xuid}`" :key="person.xuid" as="button" type="button" class="library-row library-row--person" @click="router.push({ name: 'my-friend', params: { targetXuid: person.xuid }, query: { from: route.fullPath } })">
        <img v-if="person.avatarUrl" :src="person.avatarUrl" alt="" loading="lazy">
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
