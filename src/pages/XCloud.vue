<script setup lang="ts">
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { navigationEngine } from '@/navigation/core'
import { FOCUSABLE_SELECTOR } from '@/navigation/core/engine'
import { playNavSound, triggerNavHaptic } from '@/navigation/core/haptics'
import { usePageFocus } from '@/navigation/core/usePageFocus'
import { Focusable } from '@/navigation/core/vue'
import { resolveUiDensity } from '../app/ui-density'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import GameCard from '../components/common/GameCard.vue'
import HorizontalListRail from '../components/common/HorizontalListRail.vue'
import PageUnavailable from '../components/common/PageUnavailable.vue'
import { useShellPageHints } from '../components/layout/shell-context'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { events } from '../services/events'
import { rpc } from '../services/rpc'
import { devWarn } from '../shared/dev-log'
import { CatalogRequestState } from './xcloud-catalog-state'

type XcloudCatalogPayload = Awaited<ReturnType<typeof rpc.data.getXcloudTitles>>
type XcloudTitle = XcloudCatalogPayload['titles'][number]

interface XcloudGridCardViewModel {
  title: XcloudTitle
  nodeId: string
  imageUrl: string
}

const pageRef = ref<HTMLElement | null>(null)
const { t, locale } = useI18n()
const router = useRouter()

const isLoading = ref(false)
const isRefreshing = ref(false)
const appLevel = ref(0)
const titles = ref<XcloudTitle[]>([])
const cacheState = ref<XcloudCatalogPayload['cacheState']>('miss')
const updatedAt = ref<number | undefined>(undefined)
const loadError = ref('')
const hasLoaded = ref(false)
const searchKeyword = ref('')
const contentRef = ref<HTMLElement | null>(null)
const activeSection = ref('xcloud-recent')
let contentScroller: HTMLElement | null = null
let searchScroll = 0
const viewportWidth = ref(typeof window === 'undefined' ? 1440 : window.innerWidth)
const renderedTitleCount = ref(0)
const loadMoreSentinelRef = ref<HTMLElement | null>(null)
let loadMoreObserver: IntersectionObserver | null = null
let disposeTabSwitch: (() => void) | undefined
let disposeCatalogUpdated: (() => void) | undefined
let disposeAuthChanged: (() => void) | undefined
const catalogRequests = new CatalogRequestState()
const pageFocus = usePageFocus(() => pageRef.value, () => 'catalog', () => SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud)

// LT/RT 区域导航的 section 标识符
const XCLOUD_SECTION_IDS = ['xcloud-recent', 'xcloud-new', 'xcloud-all'] as const

const normalizedSearchKeyword = computed(() => searchKeyword.value.trim().toLowerCase())
const isSearching = computed(() => normalizedSearchKeyword.value !== '')
const hasFullAccess = computed(() => appLevel.value >= 2)
const uiDensity = computed(() => resolveUiDensity(viewportWidth.value))
const showInitialLoading = computed(() => isLoading.value && titles.value.length === 0 && !hasLoaded.value)
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

useShellPageHints(() => ({
  regionLabel: hasFullAccess.value && !isSearching.value && titles.value.length > 0 ? t('shell.hints.regions') : undefined,
  error: !!loadError.value,
  status: loadError.value || (isRefreshing.value ? t('xcloudPage.refreshingHint') : isSearching.value ? resultCountLabel.value : cacheState.value === 'stale' && updatedAtLabel.value ? t('xcloudPage.cachedHint', { time: updatedAtLabel.value }) : resultCountLabel.value),
}))

function getVisibleSections(): HTMLElement[] {
  return XCLOUD_SECTION_IDS.map(id => document.querySelector<HTMLElement>(`[data-nav-section="${id}"]`)).filter((element): element is HTMLElement => element !== null)
}

function handleContentScroll(): void {
  if (!contentScroller)
    return
  const boundary = contentScroller.getBoundingClientRect().top + 24
  const sections = getVisibleSections()
  const last = sections.filter(section => section.getBoundingClientRect().top <= boundary).at(-1) ?? sections[0]
  if (last)
    activeSection.value = last.dataset.navSection ?? 'xcloud-all'
}
function setupContentScroller(): void {
  contentScroller?.removeEventListener('scroll', handleContentScroll)
  contentScroller = contentRef.value
  contentScroller?.addEventListener('scroll', handleContentScroll, { passive: true })
  handleContentScroll()
}
function jumpToSection(id: string): void {
  const section = getVisibleSections().find(section => section.dataset.navSection === id)
  if (!section || !contentScroller)
    return
  const offset = section.getBoundingClientRect().top - contentScroller.getBoundingClientRect().top + contentScroller.scrollTop - 16
  contentScroller.scrollTo({ top: Math.max(0, offset), behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' })
  const first = section.querySelector<HTMLElement>(`.horizontal-list-rail__scroller ${FOCUSABLE_SELECTOR}`) ?? section.querySelector<HTMLElement>(FOCUSABLE_SELECTOR)
  if (first)
    navigationEngine.focusElement(first, false, true)
  activeSection.value = id
}
watch(contentRef, setupContentScroller, { flush: 'post' })
// 进入搜索时同步保存位置，恢复安排在列表 DOM 更新完成后。
watch(isSearching, (searching) => {
  if (searching && contentScroller)
    searchScroll = contentScroller.scrollTop
}, { flush: 'sync' })
watch(isSearching, async (searching) => {
  await nextTick()
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
  if (contentScroller && isSearching.value === searching)
    contentScroller.scrollTop = searching ? 0 : searchScroll
}, { flush: 'post' })

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
  hasLoaded.value = true
  loadError.value = ''
}

function clearCatalog(): void {
  catalogRequests.clear()
  titles.value = []
  cacheState.value = 'miss'
  updatedAt.value = undefined
  loadError.value = ''
  hasLoaded.value = false
  isLoading.value = false
  isRefreshing.value = false
}

watch(locale, () => {
  clearCatalog()
  void loadTitles()
})

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

  const generation = catalogRequests.capture()
  try {
    const [catalogPayload] = await Promise.allSettled([forceRefresh ? rpc.data.refreshXcloudTitles() : rpc.data.getXcloudTitles()])
    const authState = await rpc.auth.getState()
    if (!catalogRequests.isCurrent(generation))
      return
    appLevel.value = authState.appLevel
    if (catalogPayload.status === 'rejected')
      throw catalogPayload.reason
    if (catalogRequests.acceptResponse(generation, catalogPayload.value))
      applyCatalogPayload(catalogPayload.value)
  }
  catch (error) {
    if (!catalogRequests.isCurrent(generation))
      return
    devWarn('[XCloud] load titles failed:', error)
    loadError.value = error instanceof Error && error.message ? error.message : t('xcloudPage.loadFailed')
    if (titles.value.length === 0) {
      titles.value = []
      cacheState.value = 'miss'
      updatedAt.value = undefined
    }
    isRefreshing.value = false
  }
  finally {
    if (catalogRequests.isCurrent(generation)) {
      hasLoaded.value = true
      isLoading.value = false
    }
  }
}

onMounted(() => {
  handleResize()
  window.addEventListener('resize', handleResize)
  disposeCatalogUpdated = events.on('data.xcloudCatalogUpdated', (payload) => {
    if (catalogRequests.acceptEvent(payload))
      applyCatalogPayload(payload)
  })
  disposeAuthChanged = events.on('auth.stateChanged', (state) => {
    appLevel.value = state.appLevel
    if (!state.isAuthenticated || state.isAuthenticating)
      clearCatalog()
    else
      void loadTitles()
  })
  void loadTitles()
  void nextTick(setupLoadMoreObserver)
  registerTabSwitch()
  void nextTick(setupContentScroller)
})

onActivated(() => {
  if (!isLoading.value) {
    void loadTitles()
  }
  void nextTick(setupLoadMoreObserver)
  registerTabSwitch()
  void nextTick(setupContentScroller)
})

onDeactivated(() => {
  contentScroller?.removeEventListener('scroll', handleContentScroll)
  loadMoreObserver?.disconnect()
  unregisterTabSwitch()
})

onBeforeUnmount(() => {
  contentScroller?.removeEventListener('scroll', handleContentScroll)
  window.removeEventListener('resize', handleResize)
  loadMoreObserver?.disconnect()
  loadMoreObserver = null
  if (disposeCatalogUpdated !== undefined) {
    disposeCatalogUpdated()
    disposeCatalogUpdated = undefined
  }
  disposeAuthChanged?.()
  unregisterTabSwitch()
})

// 注册 LT/RT 区域切换
function registerTabSwitch(): void {
  if (disposeTabSwitch !== undefined)
    return

  disposeTabSwitch = navigationEngine.onTabSwitch((direction) => {
    if (!pageFocus.isActive() || isSearching.value || !navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell))
      return
    // 动态构建当前可见的 section 列表
    const visibleSections = XCLOUD_SECTION_IDS
      .map(id => document.querySelector(`[data-nav-section="${id}"]`) as HTMLElement | null)
      .filter((el): el is HTMLElement => el !== null)

    if (visibleSections.length === 0)
      return

    // 查找当前焦点所在 section 的索引
    const focused = navigationEngine.getCurrentFocus()
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

    playNavSound('move')
    triggerNavHaptic('move')
    jumpToSection(visibleSections[nextIndex].dataset.navSection ?? 'xcloud-all')
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
  <section ref="pageRef" class="xcloud-page ui-page-shell" :aria-label="t('xcloudPage.ariaLabel')">
    <div v-if="showInitialLoading" class="xcloud-page__loading">
      <BrandedLoading :label="t('xcloudPage.loading')" />
    </div>

    <PageUnavailable
      v-else-if="!hasFullAccess"
      :id="SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud"
      :title="t('xcloudPage.limitedTitle')"
      :body="loadError || t('xcloudPage.limitedBody')"
      :action-label="t('xcloudPage.actions.refresh')"
      :busy="isLoading || isRefreshing"
      :error="!!loadError"
      @retry="handlePrimaryAction"
    />

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
          <div class="xcloud-page__search-actions">
            <label class="xcloud-page__search-shell" :aria-label="t('xcloudPage.searchLabel')">
              <span class="xcloud-page__search-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24" class="xcloud-page__search-svg" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
                  <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 4 4" />
                </svg>
              </span>
              <input
                v-model="searchKeyword"
                type="search"
                class="xcloud-page__search-input"
                :placeholder="t('xcloudPage.searchPlaceholder')"
                :aria-label="t('xcloudPage.searchLabel')"
                data-focusable="true"
                @focus="navigationEngine.focusElement($event.target as HTMLInputElement, false)"
              >
            </label>
            <Focusable id="xcloud.refresh" as="button" type="button" class="xcloud-page__refresh" :aria-label="t('xcloudPage.actions.refresh')" :title="t('xcloudPage.actions.refresh')" :aria-busy="isRefreshing" @click="loadTitles(true)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" :class="{ 'xcloud-page__refresh-icon--spinning': isRefreshing }"><path d="M20 7v5h-5M4 17v-5h5" /><path d="M6.2 6.2A8 8 0 0 1 20 12M4 12a8 8 0 0 0 13.8 5.8" /></svg>
            </Focusable>
          </div>
        </div>
        <nav v-if="!isSearching && titles.length" class="xcloud-page__jumps" :aria-label="t('shell.hints.regions')">
          <Focusable v-if="recentCards.length" id="xcloud.jump.recent" as="button" type="button" :aria-pressed="activeSection === 'xcloud-recent'" @click="jumpToSection('xcloud-recent')">
            {{ t('xcloudPage.sections.recent.title') }}
          </Focusable>
          <Focusable v-if="newCards.length" id="xcloud.jump.new" as="button" type="button" :aria-pressed="activeSection === 'xcloud-new'" @click="jumpToSection('xcloud-new')">
            {{ t('xcloudPage.sections.new.title') }}
          </Focusable>
          <Focusable id="xcloud.jump.all" as="button" type="button" :aria-pressed="activeSection === 'xcloud-all'" @click="jumpToSection('xcloud-all')">
            {{ t('xcloudPage.sections.all.title') }}
          </Focusable>
        </nav>
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
          {{ loadError || (
            isSearching
              ? t('xcloudPage.emptySearchBody', { keyword: searchKeyword })
              : t('xcloudPage.emptyBody')
          ) }}
        </p>

        <Focusable
          :id="SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud"
          as="button"
          type="button"
          class="xcloud-page__action-button ui-action-button ui-action-button--brand"
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

      <div v-else ref="contentRef" class="xcloud-page__content" data-nav-scroll-container>
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
  height: 100%;
  min-height: 0;
  overflow: hidden;
  overflow-anchor: none;
}

.xcloud-page__loading {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 0;
}

.xcloud-page__header { display: flex; flex: 0 0 auto; flex-direction: column; gap: var(--space-16); padding: var(--space-16) var(--space-8); background: transparent; }
.xcloud-page__jumps { display: flex; align-items: center; gap: var(--space-8); flex-wrap: wrap; }
.xcloud-page__jumps button { min-height: 32px; padding: var(--space-4) var(--space-12); border: 1px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: var(--color-text-secondary); font: inherit; font-size: 13px; line-height: 18px; cursor: pointer; }
.xcloud-page__jumps button[aria-pressed='true'] { background: var(--ui-choice-selected-bg); color: var(--color-text-primary); border-color: var(--ui-choice-selected-border); }
.xcloud-page__jumps button:hover { background: var(--ui-material-hover); }
.xcloud-page__jumps button[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }

.xcloud-page__toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 24px;
  padding: 0;
}

.xcloud-page__title-block {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.xcloud-page__title {
  font-size: 28px;
  line-height: 1.08;
  font-weight: 500;
  letter-spacing: -0.02em;
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

.xcloud-page__search-actions { display: flex; align-items: center; gap: var(--space-12); flex: 0 1 344px; min-width: 0; }
.xcloud-page__refresh { display: grid; place-items: center; flex: 0 0 36px; width: 36px; height: 36px; padding: 8px; border: 1px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: var(--color-text-primary); cursor: pointer; }
.xcloud-page__refresh svg { width: 20px; height: 20px; }
.xcloud-page__refresh:hover { background: var(--ui-material-hover); }
.xcloud-page__refresh[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.xcloud-page__refresh[aria-busy='true'] { opacity: .5; cursor: progress; }
.xcloud-page__refresh-icon--spinning { animation: xcloud-refresh-spin 900ms linear infinite; }
@keyframes xcloud-refresh-spin { to { transform: rotate(360deg); } }

.xcloud-page__search-shell {
  position: relative;
  display: flex;
  align-items: center;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 40px;
  padding: 0 var(--ui-xcloud-search-padding-inline);
  border: 1px solid var(--color-border-subtle);
  border-radius: var(--ui-radius-md);
  background: color-mix(in srgb, var(--ui-panel-bottom) 70%, transparent);
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
  font-size: 14px;
  line-height: 1.2;
}

.xcloud-page__search-input::placeholder {
  color: var(--color-text-tertiary);
}

.xcloud-page__search-shell:focus-within { box-shadow: var(--shadow-xbox-focus); }
.xcloud-page__search-input:focus-visible { outline: none; }

.xcloud-page__content {
  display: flex;
  flex-direction: column;
  gap: var(--space-24);
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: var(--space-12) var(--space-8) var(--space-24);
  scroll-padding: var(--space-12) var(--space-8) var(--space-24);
}

.xcloud-page__rail {
  /* Offset inside padding if needed, but HorizontalListRail handles its own scroll padding */
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
  padding: 0 var(--space-8);
}

.xcloud-page__grid-title {
  margin: 0;
  font-size: var(--ui-rail-title-size);
  font-weight: 500;
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
  padding: var(--space-12) var(--space-8); /* 为首尾卡片保留完整焦点环。 */
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

.xcloud-page__state--logged-out {
  flex: 1 1 auto;
  width: min(100%, var(--ui-login-content-width));
  margin: auto;
  align-items: center;
  text-align: center;
}

.xcloud-page__state--logged-out .ui-page-title,
.xcloud-page__state--logged-out .ui-page-body {
  width: min(100%, var(--ui-login-desc-width));
}

.xcloud-page__action-button {
  min-width: min(100%, var(--ui-login-signin-width));
  min-height: var(--ui-xcloud-action-min-height);
  margin: 0 auto;
  cursor: pointer;
}

.xcloud-page__state:not(.xcloud-page__state--logged-out) .xcloud-page__action-button {
  min-width: var(--ui-xcloud-action-min-width);
  margin: 0;
}

.xcloud-page__state:not(.xcloud-page__state--logged-out) .xcloud-page__action-button.ui-action-button {
  background: var(--btn-primary-bg);
  color: var(--btn-primary-text);
}

.xcloud-page__state:not(.xcloud-page__state--logged-out) .xcloud-page__action-button.ui-action-button--brand {
  border-color: var(--btn-border);
}

.xcloud-page__state:not(.xcloud-page__state--logged-out) .xcloud-page__action-button:hover {
  background: var(--btn-primary-bg-hover);
  filter: brightness(1.01);
}

.xcloud-page__state:not(.xcloud-page__state--logged-out) .xcloud-page__action-button[data-focused='true'] {
  box-shadow: var(--shadow-xbox-focus);
}

.xcloud-page__state--logged-out .xcloud-page__action-button {
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.xcloud-page__state--logged-out .xcloud-page__action-button[data-focused='true'] {
  transform: none;
  box-shadow: var(--shadow-xbox-focus);
}

.xcloud-page__state--logged-out .xcloud-page__action-button:disabled {
  opacity: 0.62;
  cursor: not-allowed;
  transform: none;
}

@media(max-width:700px) { .xcloud-page__toolbar { flex-direction: column; align-items: stretch; gap: var(--space-16); } .xcloud-page__title { font-size: 24px; } .xcloud-page__search-actions { flex-basis: auto; width: 100%; } .xcloud-page__results { white-space: normal; } .xcloud-page__jumps { gap: var(--space-4); } }
</style>
