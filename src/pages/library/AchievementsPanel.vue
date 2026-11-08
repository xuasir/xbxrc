<script setup lang="ts">
import type { LibraryAchievement } from '@/shared/rpc/library'
import { computed, onDeactivated, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import RemoteImage from '@/components/common/RemoteImage.vue'
import SettingInputControl from '@/components/settings/SettingInputControl.vue'
import SettingModalShell from '@/components/settings/SettingModalShell.vue'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import { achievementKey, formatDate, useLibraryFocusFallback } from './library-view'
import LibraryRefresh from './LibraryRefresh.vue'
import { libraryEpoch, useLibraryPage } from './use-library-query'

const props = withDefaults(defineProps<{ xboxTitleId: string, targetXuid?: string }>(), { targetXuid: '' })
const { locale } = useI18n()
const language = computed(() => locale.value === 'zh' ? 'zh-CN' : 'en-US')
const own = useLibraryPage(() => `achievements:self:${props.xboxTitleId}:${language.value}`, continuation => rpc.data.getAchievements({ xboxTitleId: props.xboxTitleId, language: language.value, continuation }), achievementKey)
const friend = useLibraryPage(() => props.targetXuid ? `achievements:${props.targetXuid}:${props.xboxTitleId}:${language.value}` : '', continuation => rpc.data.getAchievements({ xboxTitleId: props.xboxTitleId, targetXuid: props.targetXuid, language: language.value, continuation }), achievementKey)
const revealed = ref(new Set<string>())
const search = ref('')
const filter = ref('all')
const detail = ref<LibraryAchievement | null>(null)
onDeactivated(() => {
  detail.value = null
})
watch(libraryEpoch, () => {
  detail.value = null
  revealed.value.clear()
})
const ownMap = computed(() => new Map((own.state.value.value?.items ?? []).map(a => [achievementKey(a), a])))
const friendMap = computed(() => new Map((friend.state.value.value?.items ?? []).map(a => [achievementKey(a), a])))
const rows = computed(() => [...new Map([...(own.state.value.value?.items ?? []), ...(props.targetXuid ? friend.state.value.value?.items ?? [] : [])].map(a => [achievementKey(a), a])).values()].filter(a => name(a).toLowerCase().includes(search.value.toLowerCase()) && (filter.value === 'all' || (filter.value === 'unlocked' ? ownMap.value.get(achievementKey(a))?.isUnlocked : !ownMap.value.get(achievementKey(a))?.isUnlocked))))
useLibraryFocusFallback(() => ['game.achievements.filter', ...rows.value.map(a => `achievement.${achievementKey(a)}`), ...(own.state.value.value?.continuation || friend.state.value.value?.continuation ? ['game.achievements.more'] : [])])
function secret(a: LibraryAchievement) {
  return a.isSecret && !ownMap.value.get(achievementKey(a))?.isUnlocked && !revealed.value.has(achievementKey(a))
}
function name(a: LibraryAchievement) {
  return secret(a) ? '秘密成就' : a.name
}
function status(a: LibraryAchievement | undefined) {
  return !a ? '此项数据暂缺' : a.isUnlocked ? '已解锁' : `待解锁${a.progressPercentage != null ? ` · ${Math.round(a.progressPercentage)}%` : ''}`
}
async function refresh() {
  await Promise.allSettled([own.refresh(), ...(props.targetXuid ? [friend.refresh()] : [])])
}
async function more() {
  await Promise.allSettled([own.more(), ...(props.targetXuid ? [friend.more()] : [])])
}
</script>

<template>
  <section class="library-stack" :aria-label="targetXuid ? '成就对比' : '游戏成就'">
    <div class="library-toolbar">
      <h2 class="library-section-title">
        {{ targetXuid ? '成就对比' : '游戏成就' }}
      </h2><LibraryRefresh id="game.achievements.refresh" :busy="own.state.value.loading || friend.state.value.loading" @refresh="refresh" />
    </div>
    <p v-if="own.state.value.error" class="library-notice" role="alert">
      我的：{{ own.state.value.error }}
    </p><p v-if="targetXuid && friend.state.value.error" class="library-notice" role="alert">
      好友：{{ friend.state.value.error }}
    </p>
    <div class="library-toolbar">
      <SettingInputControl id="game.achievements.search" v-model="search" scope-id="app.shell" label="搜索成就" placeholder="搜索成就" /><Focusable id="game.achievements.filter" as="select" :value="filter" class="library-control" aria-label="成就状态" @change="filter = ($event.target as HTMLSelectElement).value">
        <option value="all">
          全部
        </option><option value="locked">
          待解锁
        </option><option value="unlocked">
          已解锁
        </option>
      </Focusable>
    </div>
    <p v-if="targetXuid" class="library-caption">
      按同一成就对比双方的解锁状态与进度。
    </p>
    <div class="library-list">
      <Focusable v-for="item in rows" :id="`achievement.${achievementKey(item)}`" :key="achievementKey(item)" as="button" class="library-row achievement-row" @click="detail = ownMap.get(achievementKey(item)) || item">
        <RemoteImage v-if="!secret(item)" class="library-thumb" :src="item.imageUrl" /><span class="library-row__body"><strong>{{ name(item) }}</strong><span class="library-caption">{{ secret(item) ? '打开详情可主动揭示' : item.isUnlocked ? item.description : item.lockedDescription || item.description }}</span><span class="library-caption"> · {{ item.gamerscore ?? '—' }} G</span></span><span class="achievement-status"><span class="library-caption">我的</span><strong>{{ status(ownMap.get(achievementKey(item))) }}</strong></span><span v-if="targetXuid" class="achievement-status"><span class="library-caption">好友</span><strong>{{ status(friendMap.get(achievementKey(item))) }}</strong></span>
      </Focusable>
    </div>
    <p v-if="own.state.value.loading || (targetXuid && friend.state.value.loading)" class="library-state" role="status">
      正在读取成就…
    </p><p v-else-if="!rows.length && !own.state.value.error && !(targetXuid && friend.state.value.error)" class="library-state">
      {{ search || filter !== 'all' ? '当前筛选结果为空' : 'Xbox 服务暂未提供此游戏的成就列表' }}
    </p>
    <Focusable v-if="own.state.value.value?.continuation || (targetXuid && friend.state.value.value?.continuation)" id="game.achievements.more" as="button" class="library-control" :disabled="own.state.value.loading || friend.state.value.loading" @click="more">
      加载更多成就
    </Focusable>
    <SettingModalShell :open="!!detail" scope-id="achievement.detail" :title="detail ? name(detail) : '成就详情'" eyebrow="成就详情" @close="detail = null">
      <template v-if="detail">
        <p class="library-copy">
          {{ detail.gamerscore ?? '—' }} G · {{ status(detail) }}
        </p><Focusable v-if="secret(detail)" id="achievement.reveal" as="button" class="library-control" @click="revealed.add(achievementKey(detail))">
          揭示秘密成就
        </Focusable><template v-else>
          <p class="library-copy">
            {{ detail.isUnlocked ? detail.description : detail.lockedDescription || detail.description }}
          </p><p v-if="detail.isUnlocked" class="library-copy">
            解锁于 {{ formatDate(detail.unlockedAt) }}
          </p><div v-for="requirement in detail.requirements" :key="requirement.id" class="library-copy">
            进度 {{ requirement.current ?? '—' }} / {{ requirement.target ?? '—' }}
          </div>
        </template>
      </template>
    </SettingModalShell>
  </section>
</template>

<style scoped>
.achievement-status { display:flex; flex-direction:column; gap:4px; min-width:88px; }
.achievement-status strong { font-size:var(--ui-text-body-sm); line-height:var(--ui-line-height-relaxed); font-weight:var(--ui-font-weight-medium); }
@media(max-width:960px) { .achievement-row { flex-wrap:wrap; } .achievement-row .library-row__body { flex-basis:calc(100% - 80px); } }
</style>
