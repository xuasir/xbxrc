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
import ExportDiagnosticsButton from '../components/common/ExportDiagnosticsButton.vue'
import GameCard from '../components/common/GameCard.vue'
import HorizontalListRail from '../components/common/HorizontalListRail.vue'
import { useShellPageHints } from '../components/layout/shell-context'
import SettingInputControl from '../components/settings/SettingInputControl.vue'
import SettingSingleSelectPopupSheet from '../components/settings/SettingSingleSelectPopupSheet.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { diagnosticIssueId } from '../services/diagnostics'
import { events } from '../services/events'
import { rpc } from '../services/rpc'
import { devWarn } from '../shared/dev-log'
import { CatalogRequestState } from './xcloud-catalog-state'
import { cloudNode, collectionTitles, entitlement, mergeFavorites, normalizeSearch, queryTitles, restoredWindow } from './xcloud/catalog-model'
import { useCloudPreferences } from './xcloud/use-cloud-preferences'
import { useCloudWait } from './xcloud/use-cloud-wait'

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

const prefs = useCloudPreferences()
const wait = useCloudWait()
const selectedProduct = ref('')
const access = ref('browse')
const sources = ref<Record<string, string>>({})
const category = ref('')
const sort = ref('name')
const filterSheet = ref<'category' | 'sort' | ''>('')
const starting = ref(false)
let disposeFocus: (() => void) | undefined
let searchRenderCount = 0
let anchorProduct = ''
const isLoading = ref(false)
const isRefreshing = ref(false)
const appLevel = ref(0)
const titles = ref<XcloudTitle[]>([])
const cacheState = ref<XcloudCatalogPayload['cacheState']>('miss')
const updatedAt = ref<number | undefined>(undefined)
const loadError = ref('')
const loadIssueId = ref<string>()
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
let pageActive = false
const catalogRequests = new CatalogRequestState()
const pageFocus = usePageFocus(() => pageRef.value, () => 'catalog', () => SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud)

// LT/RT 区域导航的 section 标识符
const XCLOUD_SECTION_IDS = ['xcloud-recent', 'xcloud-favorites', 'xcloud-new', 'xcloud-popular', 'xcloud-leaving', 'xcloud-all'] as const

const normalizedSearchKeyword = computed(() => normalizeSearch(searchKeyword.value))
const isSearching = computed(() => normalizedSearchKeyword.value !== '')
const isFiltered = computed(() => isSearching.value || category.value !== '')
const hasFullAccess = computed(() => appLevel.value >= 2)
const uiDensity = computed(() => resolveUiDensity(viewportWidth.value))
const showInitialLoading = computed(() => !hasLoaded.value && (isLoading.value || isRefreshing.value) && titles.value.length === 0)
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

const favoriteTitles = computed(() => mergeFavorites(titles.value, prefs.preferences.value.favorites, prefs.preferences.value.pinned))
const categories = computed(() => [...new Set(titles.value.flatMap(title => title.categories))].sort())
const filterOptions = computed(() => filterSheet.value === 'category'
  ? [{ value: '', label: t('xcloudUx.allCategories') }, ...categories.value.map(value => ({ value, label: value }))]
  : ['name', 'recent', 'newest'].map(value => ({ value, label: t(`xcloudUx.sort.${value}`) })))
const selectedTitle = computed(() => titles.value.find(title => title.productId === selectedProduct.value) ?? favoriteTitles.value.find(title => title.productId === selectedProduct.value))
const searchedTitles = computed(() => queryTitles(titles.value, searchKeyword.value, category.value, sort.value))
const recentTitles = computed(() => collectionTitles(titles.value, 'recentRank'))
const newTitles = computed(() => collectionTitles(titles.value, 'newRank'))
const rails = computed(() => [
  { id: 'xcloud-recent', label: t('xcloudPage.sections.recent.title'), titles: recentTitles.value },
  { id: 'xcloud-favorites', label: t('xcloudUx.favorites'), titles: favoriteTitles.value },
  { id: 'xcloud-new', label: t('xcloudPage.sections.new.title'), titles: newTitles.value },
  { id: 'xcloud-popular', label: t('xcloudUx.popular'), titles: collectionTitles(titles.value, 'popularRank') },
  { id: 'xcloud-leaving', label: t('xcloudUx.leaving'), titles: collectionTitles(titles.value, 'leavingRank') },
].filter(rail => rail.titles.length))
function badge(title: XcloudTitle) {
  if (!titles.value.some(item => item.productId === title.productId))
    return t('xcloudUx.catalogMissing')
  return entitlement(title) === 'denied' ? t('xcloudUx.accessRequired') : ''
}
function waitLabel() {
  if (wait.loading.value)
    return t('xcloudUx.waitLoading')
  if (wait.error.value)
    return t('xcloudUx.waitFailed')
  return wait.seconds.value == null ? t('xcloudUx.waitUnknown') : wait.seconds.value === 0 ? t('xcloudUx.waitReady') : t('xcloudUx.waitSeconds', { seconds: wait.seconds.value })
}
const initialRenderCount = computed(() => gridColumnCount.value * 4)
const loadMoreBatchSize = computed(() => gridColumnCount.value * 3)

const gridSourceTitles = computed(() => searchedTitles.value)
const renderedGridTitles = computed(() => gridSourceTitles.value.slice(0, renderedTitleCount.value))

const gridCards = computed<XcloudGridCardViewModel[]>(() => renderedGridTitles.value.map(title => ({
  title,
  imageUrl: resolveTitleImage(title),
  nodeId: cloudNode('all', title.productId),
})))
const resultCountLabel = computed(() =>
  t('xcloudPage.resultsCount', {
    count: gridSourceTitles.value.length,
  }),
)

const hasMoreTitles = computed(() => renderedTitleCount.value < gridSourceTitles.value.length)

watch(
  [gridSourceTitles, gridColumnCount],
  async () => {
    const focusId = navigationEngine.getCurrentFocus()?.closest('[data-cloud-product]') ? navigationEngine.getCurrentFocus()?.id : ''
    renderedTitleCount.value = restoredWindow(renderedTitleCount.value, initialRenderCount.value, gridSourceTitles.value, anchorProduct)
    await nextTick()
    if (focusId && pageFocus.isActive())
      navigationEngine.focusElement(document.getElementById(focusId), false, true)
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
  void prefs.update({ section: id })
}
watch(contentRef, setupContentScroller, { flush: 'post' })
// 进入搜索时同步保存位置，恢复安排在列表 DOM 更新完成后。
watch(isSearching, (searching) => {
  if (searching && contentScroller) {
    searchScroll = contentScroller.scrollTop
    searchRenderCount = renderedTitleCount.value
    renderedTitleCount.value = initialRenderCount.value
  }
  else if (!searching) {
    renderedTitleCount.value = Math.max(searchRenderCount, initialRenderCount.value)
  }
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

  const scrollRoot = contentRef.value
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

function openDetail(title: XcloudTitle): void {
  anchorProduct = title.productId
  pageFocus.save()
  void router.push({ name: 'cloud-game', params: { productId: title.productId } })
}
async function startStream(title: XcloudTitle): Promise<void> {
  if (starting.value)
    return
  starting.value = true
  loadError.value = ''
  const generation = catalogRequests.capture()
  try {
    const auth = await rpc.auth.getState()
    if (!auth.isAuthenticated) {
      await router.push({ name: 'login' })
      return
    }
    const current = await rpc.data.resolveCloudTitle({ productId: title.productId })
    if (!catalogRequests.isCurrent(generation))
      return
    if (entitlement(current) !== 'allowed') {
      loadError.value = t(`xcloudUx.${entitlement(current)}`)
      return
    }
    anchorProduct = title.productId
    pageFocus.save()
    await router.push({ name: 'xcloud-stream', params: { targetId: current.titleId }, query: { name: title.name } })
  }
  catch (cause) {
    if (catalogRequests.isCurrent(generation))
      loadError.value = cause instanceof Error ? cause.message : String(cause)
  }
  finally { starting.value = false }
}
function setFilter(value: string | number) {
  if (filterSheet.value === 'category')
    category.value = String(value)
  else sort.value = String(value)
  filterSheet.value = ''
  void prefs.update({ category: category.value, sort: sort.value })
}
function clearSearch(): void {
  searchKeyword.value = ''
}

function handlePrimaryAction(): void {
  if (isFiltered.value) {
    clearSearch()
    category.value = ''
    void prefs.update({ category: '' })
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
  loadError.value = payload.error ?? ''
  if (!anchorProduct && prefs.preferences.value.section && payload.titles.length && !isSearching.value) {
    void nextTick(() => {
      if (pageFocus.isActive() && !anchorProduct)
        jumpToSection(prefs.preferences.value.section)
    })
  }
  access.value = payload.access ?? 'browse'
  sources.value = payload.sources ?? {}
}

function clearCatalog(): void {
  catalogRequests.clear()
  wait.cancel()
  selectedProduct.value = ''
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
  if (isLoading.value) {
    return
  }

  const shouldBlock = !hasLoaded.value && titles.value.length === 0
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
    await prefs.load()
    if (!catalogRequests.isCurrent(generation))
      return
    appLevel.value = authState.appLevel
    category.value = prefs.preferences.value.category
    sort.value = prefs.preferences.value.sort || 'name'
    if (catalogPayload.status === 'rejected')
      throw catalogPayload.reason
    if (catalogRequests.acceptResponse(generation, catalogPayload.value))
      applyCatalogPayload(catalogPayload.value)
  }
  catch (error) {
    if (!catalogRequests.isCurrent(generation))
      return
    devWarn('[XCloud] load titles failed:', error)
    loadIssueId.value = diagnosticIssueId(error)
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
  pageActive = true
  handleResize()
  window.addEventListener('resize', handleResize)
  disposeCatalogUpdated = events.on('data.xcloudCatalogUpdated', (payload) => {
    if (catalogRequests.acceptEvent(payload))
      applyCatalogPayload(payload)
  })
  disposeAuthChanged = events.on('auth.stateChanged', (state) => {
    appLevel.value = state.appLevel
    if (!state.isAuthenticated) {
      clearCatalog()
      void loadTitles()
    }
    // 静默续期保留在途目录请求，完成云授权后允许刷新。
    else if (!state.isAuthenticating && pageActive) {
      void loadTitles()
    }
  })
  disposeFocus = navigationEngine.onFocusChange((element) => {
    if (!pageFocus.isActive())
      return
    const id = element?.closest<HTMLElement>('[data-cloud-product]')?.dataset.cloudProduct
    if (id) {
      selectedProduct.value = id
      anchorProduct = id
      const title = selectedTitle.value
      wait.focus(id, access.value === 'account' && entitlement(title) === 'allowed')
    }
    else {
      wait.cancel()
    }
  })
  void loadTitles()
  void nextTick(setupLoadMoreObserver)
  registerTabSwitch()
  void nextTick(setupContentScroller)
})

onActivated(() => {
  pageActive = true
  if (!isLoading.value) {
    void loadTitles()
  }
  void nextTick(setupLoadMoreObserver)
  registerTabSwitch()
  void nextTick(setupContentScroller)
})

onDeactivated(() => {
  pageActive = false
  contentScroller?.removeEventListener('scroll', handleContentScroll)
  loadMoreObserver?.disconnect()
  unregisterTabSwitch()
})

onBeforeUnmount(() => {
  pageActive = false
  contentScroller?.removeEventListener('scroll', handleContentScroll)
  window.removeEventListener('resize', handleResize)
  loadMoreObserver?.disconnect()
  loadMoreObserver = null
  if (disposeCatalogUpdated !== undefined) {
    disposeCatalogUpdated()
    disposeCatalogUpdated = undefined
  }
  disposeAuthChanged?.()
  disposeFocus?.()
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

    <template v-else>
      <div v-if="loadError && titles.length" class="ui-page-panel">
        <p role="alert">
          {{ loadError }}
        </p>
        <ExportDiagnosticsButton id="xcloud.cachedError.export" scope-id="app.shell" :issue-id="loadIssueId" />
      </div>
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
            <div class="xcloud-page__search-shell">
              <span class="xcloud-page__search-icon" aria-hidden="true">
                <svg viewBox="0 0 24 24" class="xcloud-page__search-svg" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
                  <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 4 4" />
                </svg>
              </span>
              <SettingInputControl
                id="xcloud.search"
                v-model="searchKeyword"
                external-focus
                scope-id="app.shell"
                type="search"
                class="xcloud-page__search-input"
                :placeholder="t('xcloudPage.searchPlaceholder')"
                :label="t('xcloudPage.searchLabel')"
              />
            </div>
            <Focusable id="xcloud.refresh" as="button" type="button" class="xcloud-page__refresh" :aria-label="t('xcloudPage.actions.refresh')" :title="t('xcloudPage.actions.refresh')" :busy="isRefreshing" @click="loadTitles(true)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" :class="{ 'xcloud-page__refresh-icon--spinning': isRefreshing }"><path d="M20 7v5h-5M4 17v-5h5" /><path d="M6.2 6.2A8 8 0 0 1 20 12M4 12a8 8 0 0 0 13.8 5.8" /></svg>
            </Focusable>
          </div>
        </div>
        <p class="xcloud-page__status" role="status">
          {{ access === 'account' ? t('xcloudUx.accountCatalog') : t('xcloudUx.browseCatalog') }} · {{ isRefreshing ? t('xcloudPage.refreshingHint') : updatedAtLabel ? t('xcloudPage.cachedHint', { time: updatedAtLabel }) : t('xcloudUx.waitingCatalog') }}
        </p>
        <p v-if="Object.values(sources).includes('unavailable')" class="xcloud-page__status" role="status">
          {{ t('xcloudUx.partialSources') }}
        </p>
        <div class="xcloud-page__jumps">
          <Focusable id="xcloud.filter.category" as="button" @click="filterSheet = 'category'">
            {{ category || t('xcloudUx.allCategories') }}
          </Focusable>
          <Focusable id="xcloud.filter.sort" as="button" @click="filterSheet = 'sort'">
            {{ t(`xcloudUx.sort.${sort}`) }}
          </Focusable>
          <Focusable v-if="isSearching || category" id="xcloud.clear" as="button" @click="searchKeyword = ''; category = ''; prefs.update({ category: '' })">
            {{ t('xcloudUx.clearFilters') }}
          </Focusable>
        </div>
        <nav v-if="!isSearching && (titles.length || rails.length)" class="xcloud-page__jumps" :aria-label="t('shell.hints.regions')">
          <Focusable v-for="rail in rails" :id="`xcloud.jump.${rail.id}`" :key="rail.id" as="button" type="button" :aria-pressed="activeSection === rail.id" @click="jumpToSection(rail.id)">
            {{ rail.label }}
          </Focusable>
          <Focusable id="xcloud.jump.all" as="button" type="button" :aria-pressed="activeSection === 'xcloud-all'" @click="jumpToSection('xcloud-all')">
            {{ t('xcloudPage.sections.all.title') }}
          </Focusable>
        </nav>
      </header>

      <section
        v-if="gridSourceTitles.length === 0 && (isSearching || category || !favoriteTitles.length)"
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

        <ExportDiagnosticsButton v-if="loadError" id="xcloud.error.export" scope-id="app.shell" :issue-id="loadIssueId" />
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
        <template v-if="!isSearching && !category">
          <HorizontalListRail v-for="rail in rails" :key="rail.id" class="xcloud-page__rail" :data-nav-section="rail.id" :title="rail.label" :hint="t('xcloudPage.resultsCount', { count: rail.titles.length })" :aria-label="rail.label">
            <GameCard v-for="title in rail.titles" :id="cloudNode(rail.id, title.productId)" :key="title.productId" scope-id="app.shell" :data-cloud-product="title.productId" :title="title.name" :image-url="title.tileImageUrl" :image-sources="[title.posterImageUrl]" :actions="true" :favorite="prefs.preferences.value.favorites.some(item => item.productId === title.productId)" :play-busy="starting && selectedProduct === title.productId" :play-disabled="entitlement(title) === 'denied'" :play-label="t('xcloudPage.actions.playSelected', { name: title.name })" :favorite-label="t(prefs.preferences.value.favorites.some(item => item.productId === title.productId) ? 'xcloudUx.unfavorite' : 'xcloudUx.favorite')" :meta="wait.product.value === title.productId ? waitLabel() : ''" :badge="badge(title)" :aria-label="t('xcloudUx.detailsFor', { name: title.name })" @play="startStream(title)" @favorite="prefs.favorite(title)" @select="openDetail(title)" />
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
              :aria-label="t('xcloudUx.detailsFor', { name: card.title.name })"
              :data-cloud-product="card.title.productId"
              :image-sources="[card.title.posterImageUrl]"
              :actions="true" :favorite="prefs.preferences.value.favorites.some(item => item.productId === card.title.productId)" :play-busy="starting && selectedProduct === card.title.productId" :play-disabled="entitlement(card.title) === 'denied'" :play-label="t('xcloudPage.actions.playSelected', { name: card.title.name })" :favorite-label="t(prefs.preferences.value.favorites.some(item => item.productId === card.title.productId) ? 'xcloudUx.unfavorite' : 'xcloudUx.favorite')" :meta="wait.product.value === card.title.productId ? waitLabel() : ''" :badge="badge(card.title)" @play="startStream(card.title)" @favorite="prefs.favorite(card.title)"
              @select="openDetail(card.title)"
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
      <p v-if="prefs.error.value" role="alert">
        {{ prefs.error.value }}
      </p>
    </template>
    <SettingSingleSelectPopupSheet :open="!!filterSheet" scope-id="xcloud.filter" :eyebrow="t('xcloudPage.toolbar.title')" :title="t(filterSheet === 'category' ? 'xcloudUx.category' : 'xcloudUx.sortLabel')" :options="filterOptions" :current-value="filterSheet === 'category' ? category : sort" @close="filterSheet = ''" @select="setFilter" />
  </section>
</template>

<style scoped>
.xcloud-page__status { font-size: 12px; line-height: 1.5; color: var(--color-text-secondary); margin: 0; }
.xcloud-page__selection { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 12px; border-top: 1px solid var(--color-border-subtle); }
.xcloud-page__selection strong { font-size: 14px; }
.xcloud-page__selection p { font-size: 12px; color: var(--color-text-secondary); margin-top: 4px; }
.xcloud-page__selection-actions { display: flex; flex-wrap: wrap; gap: 8px; }
@media(max-width:700px) { .xcloud-page__selection { flex-direction: column; align-items: stretch; } }

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

.xcloud-page__header { display: flex; flex: 0 0 auto; flex-direction: column; gap: var(--space-12); padding: var(--ui-focus-safe-inset); background: transparent; }
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

.xcloud-page__search-shell :deep(.setting-input-control__field-shell) { margin-left: 10px; }
.xcloud-page__search-shell :deep(.xcloud-page__search-input) {
  flex: 1 1 auto;
  min-width: 0;
  height: 40px;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--color-text-primary);
  font-size: 14px;
  line-height: 1.2;
}

.xcloud-page__search-shell :deep(.xcloud-page__search-input::placeholder) {
  color: var(--color-text-tertiary);
}

.xcloud-page__search-shell :deep(.xcloud-page__search-input:focus-visible) { outline: none; }
/* 搜索图标与输入框共用外层焦点环，内层保留导航和编辑合同。 */
.xcloud-page__search-shell :deep(.setting-input-control__field[data-focused='true']) { box-shadow: none; }

.xcloud-page__content {
  display: flex;
  flex-direction: column;
  gap: var(--space-16);
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: var(--ui-focus-safe-inset) var(--ui-focus-safe-inset) var(--space-16);
  scroll-padding: var(--ui-focus-safe-inset) var(--ui-focus-safe-inset) var(--space-16);
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
