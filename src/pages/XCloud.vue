<script setup lang="ts">
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { navigationEngine } from '@/navigation/core'
import { FOCUSABLE_SELECTOR } from '@/navigation/core/engine'
import { playNavSound, triggerNavHaptic } from '@/navigation/core/haptics'
import { Focusable } from '@/navigation/core/vue'
import { resolveUiDensity } from '../app/ui-density'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import GameCard from '../components/common/GameCard.vue'
import HorizontalListRail from '../components/common/HorizontalListRail.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { events } from '../services/events'
import { rpc } from '../services/rpc'

type XcloudCatalogPayload = Awaited<ReturnType<typeof rpc.data.getXcloudTitles>>
type XcloudTitle = XcloudCatalogPayload['titles'][number]

interface XcloudGridCardViewModel {
  title: XcloudTitle
  nodeId: string
  imageUrl: string
}

const { t } = useI18n()
const router = useRouter()

const isLoading = ref(false)
const isRefreshing = ref(false)
const appLevel = ref(0)
const titles = ref<XcloudTitle[]>([])
const cacheState = ref<XcloudCatalogPayload['cacheState']>('miss')
const updatedAt = ref<number | undefined>(undefined)
const searchKeyword = ref('')
const viewportWidth = ref(typeof window === 'undefined' ? 1440 : window.innerWidth)
const renderedTitleCount = ref(0)
const loadMoreSentinelRef = ref<HTMLElement | null>(null)
let loadMoreObserver: IntersectionObserver | null = null
let disposeTabSwitch: (() => void) | undefined
let disposeCatalogUpdated: (() => void) | undefined

// LT/RT 区域导航的 section 标识符
const XCLOUD_SECTION_IDS = ['xcloud-recent', 'xcloud-new', 'xcloud-all'] as const

const normalizedSearchKeyword = computed(() => searchKeyword.value.trim().toLowerCase())
const isSearching = computed(() => normalizedSearchKeyword.value !== '')
const hasFullAccess = computed(() => appLevel.value >= 2)
const uiDensity = computed(() => resolveUiDensity(viewportWidth.value))
const showInitialLoading = computed(() => isLoading.value && titles.value.length === 0)
const updatedAtLabel = computed(() => {
  if (updatedAt.value === undefined) {
    return ''
  }

  return new Date(updatedAt.value).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  })
})

const gridColumnCount = computed(() => {
  if (uiDensity.value === 'comfortable') {
    return 6
  }
  if (uiDensity.value === 'standard') {
    return 5
  }
  if (uiDensity.value === 'compact') {
    return 4
  }
  return 2
})

function resolveTitleImage(title: XcloudTitle): string {
  return title.tileImageUrl || title.posterImageUrl || ''
}

function titleMatchesSearch(title: XcloudTitle, keyword: string): boolean {
  if (keyword === '') {
    return true
  }

  const searchableText = [title.name, title.publisherName, title.description, ...title.categories]
    .join(' ')
    .toLowerCase()

  return searchableText.includes(keyword)
}

const searchedTitles = computed(() =>
  titles.value.filter(title => titleMatchesSearch(title, normalizedSearchKeyword.value)),
)

const recentTitles = computed(() => titles.value.filter(title => title.isRecentlyPlayed))
const newTitles = computed(() => titles.value.filter(title => title.isNew))

const initialRenderCount = computed(() => gridColumnCount.value * 4)
const loadMoreBatchSize = computed(() => gridColumnCount.value * 3)

const gridSourceTitles = computed(() => isSearching.value ? searchedTitles.value : titles.value)
const renderedGridTitles = computed(() => gridSourceTitles.value.slice(0, renderedTitleCount.value))

const gridCards = computed<XcloudGridCardViewModel[]>(() =>
  renderedGridTitles.value.map((title, index) => ({
    title,
    imageUrl: resolveTitleImage(title),
    nodeId: `xcloud.grid.all.${index}.${title.productId}`,
  })),
)

const recentCards = computed<XcloudGridCardViewModel[]>(() =>
  recentTitles.value.map((title, index) => ({
    title,
    imageUrl: resolveTitleImage(title),
    nodeId: `xcloud.rail.recent.${index}.${title.productId}`,
  })),
)

const newCards = computed<XcloudGridCardViewModel[]>(() =>
  newTitles.value.map((title, index) => ({
    title,
    imageUrl: resolveTitleImage(title),
    nodeId: `xcloud.rail.new.${index}.${title.productId}`,
  })),
)

const resultCountLabel = computed(() =>
  t('xcloudPage.resultsCount', {
    count: gridSourceTitles.value.length,
  }),
)

const hasMoreTitles = computed(() => renderedTitleCount.value < gridSourceTitles.value.length)

watch(
  [gridSourceTitles, gridColumnCount],
  async () => {
    renderedTitleCount.value = Math.min(gridSourceTitles.value.length, initialRenderCount.value)
    await nextTick()
    setupLoadMoreObserver()
  },
  { immediate: true },
)

function handleResize(): void {
  viewportWidth.value = window.innerWidth
}

function loadMoreTitles(): void {
  if (!hasMoreTitles.value) {
    return
  }

  renderedTitleCount.value = Math.min(
    gridSourceTitles.value.length,
    renderedTitleCount.value + loadMoreBatchSize.value,
  )
}

function setupLoadMoreObserver(): void {
  loadMoreObserver?.disconnect()
  loadMoreObserver = null

  const sentinelElement = loadMoreSentinelRef.value
  if (sentinelElement === null || !hasMoreTitles.value) {
    return
  }

  const scrollRoot = document.querySelector('.app-shell__content')
  loadMoreObserver = new IntersectionObserver(
    (entries) => {
      if (entries.some(entry => entry.isIntersecting)) {
        loadMoreTitles()
      }
    },
    {
      root: scrollRoot instanceof HTMLElement ? scrollRoot : null,
      rootMargin: '240px 0px',
      threshold: 0.01,
    },
  )
  loadMoreObserver.observe(sentinelElement)
}

function startStream(title: XcloudTitle): void {
  if (title.titleId.trim() === '') {
    return
  }

  void router.push({
    name: 'xcloud-stream',
    params: {
      targetId: title.titleId,
    },
    query: {
      name: title.name,
    },
  })
}

function clearSearch(): void {
  searchKeyword.value = ''
}

function handlePrimaryAction(): void {
  if (normalizedSearchKeyword.value !== '') {
    clearSearch()
    return
  }
  void loadTitles(true)
}

function applyCatalogPayload(payload: XcloudCatalogPayload): void {
  titles.value = Array.isArray(payload.titles) ? payload.titles : []
  cacheState.value = payload.cacheState
  updatedAt.value = payload.updatedAt
  isRefreshing.value = payload.refreshing
}

async function loadTitles(forceRefresh = false): Promise<void> {
  if (isLoading.value || (forceRefresh && isRefreshing.value)) {
    return
  }

  const shouldBlock = titles.value.length === 0
  if (shouldBlock) {
    isLoading.value = true
  }
  else if (forceRefresh) {
    isRefreshing.value = true
  }

  try {
    const [authState, catalogPayload] = await Promise.all([
      rpc.auth.getState(),
      forceRefresh ? rpc.data.refreshXcloudTitles() : rpc.data.getXcloudTitles(),
    ])
    appLevel.value = authState.appLevel
    applyCatalogPayload(catalogPayload)
  }
  catch (error) {
    console.warn('[XCloud] load titles failed:', error)
    if (titles.value.length === 0) {
      titles.value = []
      cacheState.value = 'miss'
      updatedAt.value = undefined
    }
    isRefreshing.value = false
  }
  finally {
    isLoading.value = false
  }
}

onMounted(() => {
  handleResize()
  window.addEventListener('resize', handleResize)
  disposeCatalogUpdated = events.on('data.xcloudCatalogUpdated', (payload) => {
    applyCatalogPayload(payload)
  })
  void loadTitles()
  void nextTick(setupLoadMoreObserver)
  registerTabSwitch()
})

onActivated(() => {
  if (!isLoading.value) {
    void loadTitles()
  }
  void nextTick(setupLoadMoreObserver)
  registerTabSwitch()
})

onDeactivated(() => {
  unregisterTabSwitch()
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', handleResize)
  loadMoreObserver?.disconnect()
  loadMoreObserver = null
  if (disposeCatalogUpdated !== undefined) {
    disposeCatalogUpdated()
    disposeCatalogUpdated = undefined
  }
  unregisterTabSwitch()
})

// 注册 LT/RT 区域切换
function registerTabSwitch(): void {
  if (disposeTabSwitch !== undefined) return

  disposeTabSwitch = navigationEngine.onTabSwitch((direction) => {
    // 动态构建当前可见的 section 列表
    const visibleSections = XCLOUD_SECTION_IDS
      .map(id => document.querySelector(`[data-nav-section="${id}"]`) as HTMLElement | null)
      .filter((el): el is HTMLElement => el !== null)

    if (visibleSections.length === 0) return

    // 查找当前焦点所在 section 的索引
    const focused = document.querySelector('[data-focused="true"]') as HTMLElement | null
    let currentIndex = -1
    if (focused) {
      currentIndex = visibleSections.findIndex(s => s.contains(focused))
    }

    // 计算目标索引
    const nextIndex = direction === 'next'
      ? (currentIndex === -1 ? 0 : currentIndex + 1)
      : (currentIndex === -1 ? visibleSections.length - 1 : currentIndex - 1)

    if (nextIndex < 0 || nextIndex >= visibleSections.length) {
      playNavSound('boundary')
      triggerNavHaptic('boundary')
      return
    }

    // 聚焦目标区域的第一个可聚焦元素
    const targetSection = visibleSections[nextIndex]
    const firstFocusable = targetSection.querySelector(FOCUSABLE_SELECTOR) as HTMLElement | null
    if (firstFocusable) {
      playNavSound('move')
      triggerNavHaptic('move')
      navigationEngine.focusElement(firstFocusable, false)
    }
  })
}

function unregisterTabSwitch(): void {
  if (disposeTabSwitch !== undefined) {
    disposeTabSwitch()
    disposeTabSwitch = undefined
  }
}
</script>

<template>
  <section class="xcloud-page ui-page-shell" :aria-label="t('xcloudPage.ariaLabel')">
    <div v-if="showInitialLoading" class="xcloud-page__loading">
      <BrandedLoading :label="t('xcloudPage.loading')" />
    </div>

    <section
      v-else-if="!hasFullAccess"
      class="xcloud-page__state ui-page-panel ui-page-panel--spacious"
    >
      <p class="ui-page-title">
        {{ t('xcloudPage.limitedTitle') }}
      </p>
      <p class="ui-page-body">
        {{ t('xcloudPage.limitedBody') }}
      </p>

      <Focusable
        :id="SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud"
        as="button"
        type="button"
        class="xcloud-page__action-button"
        :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
        :aria-label="t('xcloudPage.action.ariaLabel')"
        @click="handlePrimaryAction"
        >
        {{ t('xcloudPage.actions.refresh') }}
      </Focusable>
    </section>

    <template v-else>
      <header class="xcloud-page__header">
        <div class="xcloud-page__toolbar">
          <div class="xcloud-page__title-block">
            <p class="xcloud-page__title">
              {{ t('xcloudPage.toolbar.title') }}
            </p>
            <p class="xcloud-page__subtitle">
              {{ t('xcloudPage.toolbar.hint', { count: titles.length }) }}
            </p>
          </div>
          <label class="xcloud-page__search-shell" :aria-label="t('xcloudPage.searchLabel')">
            <span class="xcloud-page__search-icon" aria-hidden="true">
              <svg viewBox="0 0 24 24" class="xcloud-page__search-svg">
                <path
                  d="M10.5 4.5a6 6 0 1 0 3.789 10.652l4.279 4.279 1.414-1.414-4.279-4.279A6 6 0 0 0 10.5 4.5Z"
                  fill="currentColor"
                />
              </svg>
            </span>
            <input
              v-model="searchKeyword"
              type="search"
              class="xcloud-page__search-input"
              :placeholder="t('xcloudPage.searchPlaceholder')"
            >
          </label>
        </div>
        <p v-if="isRefreshing" class="xcloud-page__refreshing-hint">
          {{ t('xcloudPage.refreshingHint') }}
        </p>
        <p
          v-else-if="cacheState === 'stale' && updatedAtLabel !== ''"
          class="xcloud-page__refreshing-hint"
        >
          {{ t('xcloudPage.cachedHint', { time: updatedAtLabel }) }}
        </p>
      </header>

      <section
        v-if="gridSourceTitles.length === 0"
        class="xcloud-page__state ui-page-panel ui-page-panel--spacious"
      >
        <p class="ui-page-title">
          {{
            isSearching
              ? t('xcloudPage.emptySearchTitle')
              : t('xcloudPage.emptyTitle')
          }}
        </p>
        <p class="ui-page-body">
          {{
            isSearching
              ? t('xcloudPage.emptySearchBody', { keyword: searchKeyword })
              : t('xcloudPage.emptyBody')
          }}
        </p>

        <Focusable
          :id="SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud"
          as="button"
          type="button"
          class="xcloud-page__action-button"
          :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
          :aria-label="
            isSearching
              ? t('xcloudPage.actions.clearSearch')
              : t('xcloudPage.actions.refresh')
          "
          @click="handlePrimaryAction"
        >
          {{
            isSearching
              ? t('xcloudPage.actions.clearSearch')
              : t('xcloudPage.actions.refresh')
          }}
        </Focusable>
      </section>

      <div v-else class="xcloud-page__content">
        <!-- 横向轨道 (仅非搜索状态显示) -->
        <template v-if="!isSearching">
          <HorizontalListRail
            v-if="recentCards.length > 0"
            class="xcloud-page__rail"
            data-nav-section="xcloud-recent"
            :title="t('xcloudPage.sections.recent.title')"
            :hint="t('xcloudPage.sections.recent.hint', { count: recentCards.length })"
            :aria-label="t('xcloudPage.sections.recent.title')"
          >
            <GameCard
              v-for="card in recentCards"
              :id="card.nodeId"
              :key="card.nodeId"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
              :title="card.title.name"
              :image-url="card.imageUrl"
              :aria-label="t('xcloudPage.actions.playSelected', { name: card.title.name })"
              :disabled="card.title.titleId.trim() === ''"
              @select="startStream(card.title)"
            />
          </HorizontalListRail>

          <HorizontalListRail
            v-if="newCards.length > 0"
            class="xcloud-page__rail"
            data-nav-section="xcloud-new"
            :title="t('xcloudPage.sections.new.title')"
            :hint="t('xcloudPage.sections.new.hint', { count: newCards.length })"
            :aria-label="t('xcloudPage.sections.new.title')"
          >
            <GameCard
              v-for="card in newCards"
              :id="card.nodeId"
              :key="card.nodeId"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
              :title="card.title.name"
              :image-url="card.imageUrl"
              :aria-label="t('xcloudPage.actions.playSelected', { name: card.title.name })"
              :disabled="card.title.titleId.trim() === ''"
              @select="startStream(card.title)"
            />
          </HorizontalListRail>
        </template>

        <!-- 所有游戏/搜索结果 网格 -->
        <section class="xcloud-page__grid-section" data-nav-section="xcloud-all">
          <header class="xcloud-page__grid-header">
            <h2 class="xcloud-page__grid-title">
              {{ isSearching ? t('xcloudPage.searchLabel') : t('xcloudPage.sections.all.title') }}
            </h2>
            <p class="xcloud-page__results">
              {{ resultCountLabel }}
            </p>
          </header>

          <div
            class="xcloud-page__grid"
            :style="{ '--xcloud-grid-columns': String(gridColumnCount) }"
          >
            <GameCard
              v-for="card in gridCards"
              :id="card.nodeId"
              :key="card.nodeId"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
              :title="card.title.name"
              :image-url="card.imageUrl"
              :aria-label="t('xcloudPage.actions.playSelected', { name: card.title.name })"
              :disabled="card.title.titleId.trim() === ''"
              @select="startStream(card.title)"
            />

            <div
              v-if="hasMoreTitles"
              ref="loadMoreSentinelRef"
              class="xcloud-page__load-more-sentinel"
              aria-hidden="true"
            />
          </div>
        </section>
      </div>
    </template>
  </section>
</template>

<style scoped>
.xcloud-page {
  display: flex;
  flex-direction: column;
  min-height: 100%;
  padding-bottom: var(--ui-space-4xl);
}

.xcloud-page__loading {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 0;
}

.xcloud-page__header {
  display: flex;
  flex-direction: column;
  position: sticky;
  top: calc(var(--ui-app-shell-content-padding-top) * -1);
  z-index: 10;
  margin: calc(var(--ui-app-shell-content-padding-top) * -1) calc(var(--ui-page-inset) * -1) 0;
  padding: var(--ui-app-shell-content-padding-top) var(--ui-page-inset);
  background: color-mix(in srgb, var(--ui-page-bg), transparent 15%);
  backdrop-filter: blur(20px);
  border-bottom: 1px solid var(--color-divider);
}

.xcloud-page__toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  padding: 12px 0; /* 给标题和搜索框增加内部垂直居中的余地 */
}

.xcloud-page__title-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.xcloud-page__title {
  font-size: clamp(22px, 1.8vw, 30px);
  line-height: 1.08;
  font-weight: var(--ui-font-weight-bold);
  letter-spacing: -0.03em;
  color: var(--color-text-primary);
}

.xcloud-page__subtitle {
  max-width: 620px;
  font-size: 13px;
  line-height: 1.45;
  color: var(--color-text-secondary);
}

.xcloud-page__refreshing-hint {
  padding-bottom: 8px;
  font-size: 12px;
  line-height: 1.4;
  color: var(--color-text-secondary);
}

.xcloud-page__search-shell {
  position: relative;
  display: flex;
  align-items: center;
  flex: 0 0 min(100%, 360px);
  min-height: var(--ui-xcloud-search-height);
  padding: 0 var(--ui-xcloud-search-padding-inline);
  border: 1px solid var(--color-border-subtle);
  border-radius: var(--btn-radius);
  background: var(--color-surface-1);
}

.xcloud-page__search-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  color: var(--color-text-secondary);
}

.xcloud-page__search-svg {
  width: 100%;
  height: 100%;
}

.xcloud-page__search-input {
  flex: 1 1 auto;
  min-width: 0;
  margin-left: 10px;
  border: 0;
  background: transparent;
  color: var(--color-text-primary);
  font-size: clamp(15px, 1.6vw, 18px);
  line-height: 1.2;
}

.xcloud-page__search-input::placeholder {
  color: var(--color-text-tertiary);
}

.xcloud-page__search-input:focus {
  outline: none;
}

.xcloud-page__content {
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-4xl);
  margin-top: var(--ui-space-2xl);
}

.xcloud-page__rail {
  /* Offset inside padding if needed, but HorizontalListRail handles its own scroll padding */
}

.xcloud-page__rail :deep(.game-card) {
  /* The default width/height defined in GameCard are used here,
     but we ensure it doesn't get overridden by anything else */
}

.xcloud-page__grid-section {
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-md);
}

.xcloud-page__grid-header {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
  padding: 0 4px; /* match grid's padding to align text */
}

.xcloud-page__grid-title {
  margin: 0;
  font-size: var(--ui-rail-title-size);
  font-weight: var(--ui-font-weight-bold);
  line-height: 1.1;
  color: var(--color-text-primary);
}

.xcloud-page__results {
  flex: 0 0 auto;
  font-size: var(--ui-rail-hint-size);
  line-height: 1.4;
  color: var(--color-text-secondary);
  white-space: nowrap;
}

.xcloud-page__grid {
  display: grid;
  grid-template-columns: repeat(var(--xcloud-grid-columns), minmax(0, 1fr));
  gap: var(--ui-xcloud-grid-gap);
  align-items: start;
  padding: 12px 4px; /* Room for scaling at the edges */
}

.xcloud-page__load-more-sentinel {
  grid-column: 1 / -1;
  width: 100%;
  height: 1px;
}

.xcloud-page__grid :deep(.game-card) {
  width: 100%;
  height: auto;
  min-height: 0;
  padding-bottom: 0;
  aspect-ratio: 1 / 1;
  justify-self: start;
}

.xcloud-page__grid :deep(.game-card__image-shell) {
  position: absolute;
  inset: 0;
}

.xcloud-page__grid :deep(.game-card) {
  position: relative;
}

.xcloud-page__state {
  display: flex;
  flex-direction: column;
  gap: 18px;
  justify-content: center;
  align-items: flex-start;
  min-height: var(--ui-xcloud-state-min-height);
}

.xcloud-page__action-button {
  min-width: var(--ui-xcloud-action-min-width);
  min-height: var(--ui-xcloud-action-min-height);
  padding: 0 18px;
  border: 1px solid var(--btn-border);
  border-radius: var(--btn-radius);
  background: var(--btn-primary-bg);
  color: var(--btn-primary-text);
  font-size: 15px;
  line-height: 1;
  font-weight: var(--ui-font-weight-bold);
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    filter var(--ui-motion-fast);
}

.xcloud-page__action-button:hover {
  background: var(--btn-primary-bg-hover);
  filter: brightness(1.01);
}

.xcloud-page__action-button[data-focused='true'] {
  box-shadow: var(--shadow-xbox-focus);
}

:global(html[data-ui-density='compact']) .xcloud-page__toolbar,
:global(html[data-ui-density='narrow']) .xcloud-page__toolbar {
  flex-direction: column;
  align-items: stretch;
}

:global(html[data-ui-density='compact']) .xcloud-page__results,
:global(html[data-ui-density='narrow']) .xcloud-page__results {
  white-space: normal;
}

:global(html[data-ui-density='narrow']) .xcloud-page__header {
  top: calc(var(--ui-space-1) * -1);
}

:global(html[data-ui-density='narrow']) .xcloud-page__search-shell {
  flex-basis: auto;
  width: 100%;
}
</style>
