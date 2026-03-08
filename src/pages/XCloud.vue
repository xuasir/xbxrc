<script setup lang="ts">
import { Focusable } from '@spatial-navigation/vue'
import { computed, nextTick, onActivated, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { resolveUiDensity } from '../app/ui-density'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import GameCard from '../components/common/GameCard.vue'
import SpatialNavTabs from '../components/navigation/SpatialNavTabs.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'

type XcloudTitle = Awaited<ReturnType<typeof rpc.data.getXcloudTitles>>[number]
type XcloudTabKey = 'all' | 'recent' | 'new'

interface XcloudGridCardViewModel {
  title: XcloudTitle
  nodeId: string
  row: number
  col: number
  imageUrl: string
}

const XCLOUD_TAB_NODE_IDS: Record<XcloudTabKey, string> = {
  all: SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud,
  recent: 'xcloud.tabs.recent',
  new: 'xcloud.tabs.new',
}

const { t } = useI18n()
const router = useRouter()

const isLoading = ref(false)
const appLevel = ref(0)
const titles = ref<XcloudTitle[]>([])
const searchKeyword = ref('')
const activeTabKey = ref<XcloudTabKey>('all')
const viewportWidth = ref(typeof window === 'undefined' ? 1440 : window.innerWidth)
const renderedTitleCount = ref(0)
const loadMoreSentinelRef = ref<HTMLElement | null>(null)
let loadMoreObserver: IntersectionObserver | null = null

const normalizedSearchKeyword = computed(() => searchKeyword.value.trim().toLowerCase())
const hasFullAccess = computed(() => appLevel.value >= 2)
const uiDensity = computed(() => resolveUiDensity(viewportWidth.value))

const tabItems = computed(() => [
  {
    key: 'all',
    label: t('xcloudPage.tabs.all'),
    nodeId: XCLOUD_TAB_NODE_IDS.all,
  },
  {
    key: 'recent',
    label: t('xcloudPage.tabs.recent'),
    nodeId: XCLOUD_TAB_NODE_IDS.recent,
  },
  {
    key: 'new',
    label: t('xcloudPage.tabs.new'),
    nodeId: XCLOUD_TAB_NODE_IDS.new,
  },
])

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

const visibleTitles = computed(() => {
  if (activeTabKey.value === 'recent') {
    return searchedTitles.value.filter(title => title.isRecentlyPlayed)
  }
  if (activeTabKey.value === 'new') {
    return searchedTitles.value.filter(title => title.isNew)
  }
  return searchedTitles.value
})

const initialRenderCount = computed(() => gridColumnCount.value * 4)
const loadMoreBatchSize = computed(() => gridColumnCount.value * 3)
const renderedTitles = computed(() => visibleTitles.value.slice(0, renderedTitleCount.value))

const gridCards = computed<XcloudGridCardViewModel[]>(() =>
  renderedTitles.value.map((title, index) => ({
    title,
    imageUrl: resolveTitleImage(title),
    nodeId: `xcloud.grid.${activeTabKey.value}.${index}.${title.productId}`,
    row: Math.floor(index / gridColumnCount.value),
    col: index % gridColumnCount.value,
  })),
)

const currentTabNodeId = computed(() => XCLOUD_TAB_NODE_IDS[activeTabKey.value])
const resultCountLabel = computed(() =>
  t('xcloudPage.resultsCount', {
    count: visibleTitles.value.length,
  }),
)
const activeSectionTitle = computed(() => t(`xcloudPage.sections.${activeTabKey.value}.title`))
const activeSectionHint = computed(() =>
  t(`xcloudPage.sections.${activeTabKey.value}.hint`, {
    count: visibleTitles.value.length,
  }),
)
const hasMoreTitles = computed(() => renderedTitleCount.value < visibleTitles.value.length)

watch(visibleTitles, (nextTitles) => {
  if (nextTitles.length === 0 && activeTabKey.value !== 'all') {
    const fallbackTitles = searchedTitles.value
    if (fallbackTitles.length > 0) {
      activeTabKey.value = 'all'
    }
  }
})

watch(
  [visibleTitles, gridColumnCount],
  async () => {
    renderedTitleCount.value = Math.min(visibleTitles.value.length, initialRenderCount.value)
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
    visibleTitles.value.length,
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

function findCardNodeId(row: number, col: number): string | undefined {
  return gridCards.value.find(card => card.row === row && card.col === col)?.nodeId
}

function buildCardNeighbors(
  card: XcloudGridCardViewModel,
): Record<'up' | 'down' | 'left' | 'right', string | undefined> {
  return {
    up: card.row === 0 ? currentTabNodeId.value : findCardNodeId(card.row - 1, card.col),
    down: findCardNodeId(card.row + 1, card.col),
    left: findCardNodeId(card.row, card.col - 1),
    right: findCardNodeId(card.row, card.col + 1),
  }
}

function startStream(title: XcloudTitle): void {
  if (title.titleId.trim() === '') {
    return
  }

  // 临时在点击云游戏标题时打印输入配置，便于确认真实返回结构。
  void rpc.data
    .getStreamingTitleInputConfig({
      xboxTitleId: title.titleId,
    })
    .then((result) => {
      console.log('[XCloud] getStreamingTitleInputConfig result:', result)
    })
    .catch((error) => {
      console.warn('[XCloud] getStreamingTitleInputConfig failed:', error)
    })

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

async function loadTitles(forceRefresh = false): Promise<void> {
  if (isLoading.value) {
    return
  }

  if (!forceRefresh && titles.value.length > 0) {
    return
  }

  isLoading.value = true
  try {
    const [authState, xcloudTitles] = await Promise.all([
      rpc.auth.getState(),
      rpc.data.getXcloudTitles(),
    ])
    appLevel.value = authState.appLevel
    titles.value = Array.isArray(xcloudTitles) ? xcloudTitles : []
  }
  catch (error) {
    console.warn('[XCloud] load titles failed:', error)
    titles.value = []
  }
  finally {
    isLoading.value = false
  }
}

onMounted(() => {
  handleResize()
  window.addEventListener('resize', handleResize)
  void loadTitles()
  void nextTick(setupLoadMoreObserver)
})

onActivated(() => {
  if (titles.value.length === 0 && !isLoading.value) {
    void loadTitles()
  }
  void nextTick(setupLoadMoreObserver)
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', handleResize)
  loadMoreObserver?.disconnect()
  loadMoreObserver = null
})
</script>

<template>
  <section class="xcloud-page ui-page-shell" :aria-label="t('xcloudPage.ariaLabel')">
    <div v-if="isLoading" class="xcloud-page__loading">
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
        :neighbors="{ up: SPATIAL_NAV_NODE_IDS.topNav.xcloud }"
        :aria-label="t('xcloudPage.actions.refresh')"
        :on-confirm="handlePrimaryAction"
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
              {{ activeSectionTitle }}
            </p>
            <p class="xcloud-page__subtitle">
              {{ normalizedSearchKeyword === '' ? activeSectionHint : resultCountLabel }}
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

        <div class="xcloud-page__filter-bar">
          <SpatialNavTabs
            v-model:active-key="activeTabKey"
            :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
            :tabs="tabItems"
            id-prefix="xcloud.tabs"
            :up-neighbor-id="SPATIAL_NAV_NODE_IDS.topNav.xcloud"
            :down-neighbor-id="gridCards[0]?.nodeId"
            :aria-label="t('xcloudPage.tabsAriaLabel')"
          />

          <p class="xcloud-page__results">
            {{ resultCountLabel }}
          </p>
        </div>
      </header>

      <section
        v-if="gridCards.length === 0"
        class="xcloud-page__state ui-page-panel ui-page-panel--spacious"
      >
        <p class="ui-page-title">
          {{
            normalizedSearchKeyword === ''
              ? t('xcloudPage.emptyTitle')
              : t('xcloudPage.emptySearchTitle')
          }}
        </p>
        <p class="ui-page-body">
          {{
            normalizedSearchKeyword === ''
              ? t('xcloudPage.emptyBody')
              : t('xcloudPage.emptySearchBody', { keyword: searchKeyword })
          }}
        </p>

        <Focusable
          :id="SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud"
          as="button"
          type="button"
          class="xcloud-page__action-button"
          :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
          :neighbors="{ up: SPATIAL_NAV_NODE_IDS.topNav.xcloud }"
          :aria-label="
            normalizedSearchKeyword === ''
              ? t('xcloudPage.actions.refresh')
              : t('xcloudPage.actions.clearSearch')
          "
          :on-confirm="handlePrimaryAction"
          @click="handlePrimaryAction"
        >
          {{
            normalizedSearchKeyword === ''
              ? t('xcloudPage.actions.refresh')
              : t('xcloudPage.actions.clearSearch')
          }}
        </Focusable>
      </section>

      <section
        v-else
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
          :neighbors="buildCardNeighbors(card)"
          :index="{ row: card.row, col: card.col, order: card.row * gridColumnCount + card.col }"
          @select="startStream(card.title)"
        />

        <div
          v-if="hasMoreTitles"
          ref="loadMoreSentinelRef"
          class="xcloud-page__load-more-sentinel"
          aria-hidden="true"
        />
      </section>
    </template>
  </section>
</template>

<style scoped>
.xcloud-page {
  display: flex;
  flex-direction: column;
  gap: var(--ui-page-stack-gap);
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
  gap: var(--ui-xcloud-header-gap);
  position: sticky;
  top: 0;
  z-index: 1;
  padding-bottom: 2px;
}

.xcloud-page__toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 8px 0 0;
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

.xcloud-page__search-shell {
  position: relative;
  display: flex;
  align-items: center;
  flex: 0 0 min(100%, 360px);
  min-height: var(--ui-xcloud-search-height);
  padding: 0 var(--ui-xcloud-search-padding-inline);
  border: 1px solid var(--color-border-subtle);
  border-radius: var(--btn-radius);
  background: color-mix(in srgb, var(--color-surface-1) 92%, transparent);
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

.xcloud-page__filter-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-bottom: 4px;
}

.xcloud-page__results {
  flex: 0 0 auto;
  font-size: 12px;
  line-height: 1.4;
  color: var(--color-text-secondary);
  white-space: nowrap;
}

.xcloud-page__grid {
  display: grid;
  grid-template-columns: repeat(var(--xcloud-grid-columns), minmax(0, 1fr));
  gap: var(--ui-xcloud-grid-gap);
  align-items: start;
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
  border-color: var(--color-focus-ring);
  box-shadow: 0 0 0 var(--focus-ring-width) var(--color-focus-ring-outer) inset;
}

:global(html[data-ui-density='compact']) .xcloud-page__filter-bar,
:global(html[data-ui-density='narrow']) .xcloud-page__filter-bar {
  flex-direction: column;
  align-items: stretch;
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
