<script setup lang="ts">
import type { CatalogIndex } from './xcloud/catalog-index'
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { onBeforeRouteLeave, useRouter } from 'vue-router'
import { navigationEngine } from '@/navigation/core'
import { playNavSound, triggerNavHaptic } from '@/navigation/core/haptics'
import { usePageFocus } from '@/navigation/core/usePageFocus'
import { Focusable } from '@/navigation/core/vue'
import { resolveUiDensity } from '../app/ui-density'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import ExportDiagnosticsButton from '../components/common/ExportDiagnosticsButton.vue'
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
import { buildCatalogIndex, queryCatalogIndex } from './xcloud/catalog-index'
import { entitlement, normalizeSearch } from './xcloud/catalog-model'
import CloudCatalogWindow from './xcloud/CloudCatalogWindow.vue'
import { useCloudCatalog } from './xcloud/use-cloud-catalog'
import { useCloudPreferences } from './xcloud/use-cloud-preferences'
import { useCloudWait } from './xcloud/use-cloud-wait'

type XcloudCatalogPayload = Awaited<ReturnType<typeof rpc.data.getXcloudTitles>>
type XcloudTitle = XcloudCatalogPayload['titles'][number]

const pageRef = ref<HTMLElement | null>(null)
const { t, locale } = useI18n()
const router = useRouter()

const prefs = useCloudPreferences()
const catalog = useCloudCatalog()
const wait = useCloudWait()
const selectedProduct = ref('')
const access = ref('browse')
const sources = ref<Record<string, string>>({})
const category = ref('')
const sort = ref('name')
const filterSheet = ref<'category' | 'sort' | ''>('')
const starting = ref(false)
const startingProduct = ref('')
let disposeFocus: (() => void) | undefined
let anchorProduct = ''
let anchorSection = 'xcloud-all'
let anchorAction = ''
let restoreCard = false
const isLoading = ref(false)
const isRefreshing = ref(false)
const appLevel = ref(0)
const titles = shallowRef<XcloudTitle[]>([])
const cacheState = ref<XcloudCatalogPayload['cacheState']>('miss')
const updatedAt = ref<number | undefined>(undefined)
const loadError = ref('')
const loadIssueId = ref<string>()
const hasLoaded = ref(false)
const hasQueriedCatalog = ref(false)
const searchKeyword = ref('')
const contentRef = ref<HTMLElement | null>(null)
const activeSection = ref('xcloud-recent')
let contentScroller: HTMLElement | null = null
let searchScroll = 0
let browseScroll = 0
const railScrolls = new Map<string, number>()
const viewportWidth = ref(typeof window === 'undefined' ? 1440 : window.innerWidth)
const catalogIndex = shallowRef<CatalogIndex>()
const gridSourceTitles = shallowRef<XcloudTitle[]>([])
let indexedTitles: XcloudTitle[] | undefined
let indexGeneration = 0
let queryGeneration = 0
let queryTimer: ReturnType<typeof setTimeout> | undefined
let contentFrame = 0
const windows = new Map<string, InstanceType<typeof CloudCatalogWindow>>()
const demands = new Map<string, string[]>()
let demandTimer: ReturnType<typeof setTimeout> | undefined
let lastDemand = ''
let disposeTabSwitch: (() => void) | undefined
let disposeCatalogUpdated: (() => void) | undefined
let disposeAuthChanged: (() => void) | undefined
let pageActive = false
let pendingLoad: Promise<void> | undefined
let restoredSection = false
const catalogRequests = new CatalogRequestState()
const pageFocus = usePageFocus(() => pageRef.value, () => 'catalog', () => SPATIAL_NAV_NODE_IDS.pagePrimary.xcloud)

// LT/RT 区域导航的 section 标识符
const XCLOUD_SECTION_IDS = ['xcloud-recent', 'xcloud-favorites', 'xcloud-new', 'xcloud-popular', 'xcloud-leaving', 'xcloud-all'] as const

const normalizedSearchKeyword = computed(() => normalizeSearch(searchKeyword.value))
const isSearching = computed(() => normalizedSearchKeyword.value !== '')
const isFiltered = computed(() => isSearching.value || category.value !== '')
const hasFullAccess = computed(() => appLevel.value >= 2)
const uiDensity = computed(() => resolveUiDensity(viewportWidth.value))
const showInitialLoading = computed(() => (!hasLoaded.value && (isLoading.value || isRefreshing.value) && titles.value.length === 0) || (!hasQueriedCatalog.value && titles.value.length > 0))
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

const availableIds = computed(() => new Set(titles.value.map(title => title.productId)))
const favoriteIds = computed(() => new Set(prefs.preferences.value.favorites.map(title => title.productId)))
const pinnedIds = computed(() => new Set(prefs.preferences.value.pinned))
const favoriteTitles = computed(() => prefs.preferences.value.favorites.map(title => catalogIndex.value?.byId.get(title.productId) ?? { ...title, titleId: '', hasEntitlement: false, entitlementKnown: false })
  .sort((a, b) => Number(pinnedIds.value.has(b.productId)) - Number(pinnedIds.value.has(a.productId))))
const favoritesById = computed(() => new Map(favoriteTitles.value.map(title => [title.productId, title])))
const categories = computed(() => catalogIndex.value?.categories ?? [])
const filterOptions = computed(() => filterSheet.value === 'category'
  ? [{ value: '', label: t('xcloudUx.allCategories') }, ...categories.value.map(value => ({ value, label: value }))]
  : ['name', 'recent', 'newest'].map(value => ({ value, label: t(`xcloudUx.sort.${value}`) })))
const selectedTitle = computed(() => catalogIndex.value?.byId.get(selectedProduct.value) ?? favoritesById.value.get(selectedProduct.value))
const rails = computed(() => [
  { id: 'xcloud-recent', label: t('xcloudPage.sections.recent.title'), titles: catalogIndex.value?.collections.recent ?? [] },
  { id: 'xcloud-favorites', label: t('xcloudUx.favorites'), titles: favoriteTitles.value },
  { id: 'xcloud-new', label: t('xcloudPage.sections.new.title'), titles: catalogIndex.value?.collections.newest ?? [] },
  { id: 'xcloud-popular', label: t('xcloudUx.popular'), titles: catalogIndex.value?.collections.popular ?? [] },
  { id: 'xcloud-leaving', label: t('xcloudUx.leaving'), titles: catalogIndex.value?.collections.leaving ?? [] },
].filter(rail => rail.titles.length))
function waitLabel() {
  if (wait.loading.value)
    return t('xcloudUx.waitLoading')
  if (wait.error.value)
    return t('xcloudUx.waitFailed')
  return wait.seconds.value == null ? t('xcloudUx.waitUnknown') : wait.seconds.value === 0 ? t('xcloudUx.waitReady') : t('xcloudUx.waitSeconds', { seconds: wait.seconds.value })
}
const resultCountLabel = computed(() => t('xcloudPage.resultsCount', { count: gridSourceTitles.value.length }))

async function rebuildIndex() {
  if (!pageActive || indexedTitles === titles.value)
    return
  const generation = ++indexGeneration
  const source = titles.value
  const index = await buildCatalogIndex(source, catalogIndex.value, () => pageActive && generation === indexGeneration)
  if (!index || !pageActive || generation !== indexGeneration)
    return
  catalogIndex.value = index
  indexedTitles = source
  scheduleQuery(false)
}
function scheduleQuery(debounce = true) {
  clearTimeout(queryTimer)
  const generation = ++queryGeneration
  if (!pageActive || !catalogIndex.value)
    return
  const compute = async () => {
    const index = catalogIndex.value!
    const focus = navigationEngine.getCurrentFocus()
    const focusedProduct = focus?.closest<HTMLElement>('[data-cloud-product]')?.dataset.cloudProduct
    const focusAction = focus?.id.endsWith('.play') ? 'play' : focus?.id.endsWith('.favorite') ? 'favorite' : ''
    const result = await queryCatalogIndex(index, searchKeyword.value, category.value, sort.value, () => pageActive && generation === queryGeneration)
    if (!result || !pageActive || generation !== queryGeneration)
      return
    gridSourceTitles.value = result
    // 首屏等目录与查询结果就绪后一起展示，刷新期间继续呈现已提交内容。
    if (hasLoaded.value && indexedTitles === titles.value)
      hasQueriedCatalog.value = true
    await nextTick()
    if (focusedProduct && focus?.closest('[data-nav-section]')?.getAttribute('data-nav-section') === 'xcloud-all' && navigationEngine.getCurrentFocus() === focus)
      await windows.get('xcloud-all')?.reveal(focusedProduct, focusAction)
  }
  if (debounce && isSearching.value)
    queryTimer = setTimeout(() => void compute(), 120)
  else
    void compute()
}
function stopComputing() {
  indexGeneration++
  queryGeneration++
  clearTimeout(queryTimer)
  clearTimeout(demandTimer)
  cancelAnimationFrame(contentFrame)
  contentFrame = 0
}
watch(titles, () => void rebuildIndex())
watch([searchKeyword, category, sort], ([query], [previousQuery]) => scheduleQuery(query !== previousQuery))

function registerWindow(id: string, value: unknown) {
  if (value)
    windows.set(id, value as InstanceType<typeof CloudCatalogWindow>)
  else
    windows.delete(id)
  if (!value)
    demands.delete(id)
}
function prioritizeWindow(id: string, ids: string[]) {
  demands.set(id, ids)
  if (!pageActive || demandTimer)
    return
  demandTimer = setTimeout(() => {
    demandTimer = undefined
    if (!pageActive)
      return
    const productIds = [...new Set([...demands.values()].flat())].slice(0, 120)
    const key = productIds.join(',')
    if (key !== lastDemand) {
      lastDemand = key
      void rpc.data.prioritizeCloudTitles({ productIds }).catch(() => {})
    }
  }, 140)
}

useShellPageHints(() => ({
  regionLabel: hasFullAccess.value && !isSearching.value && titles.value.length > 0 ? t('shell.hints.regions') : undefined,
  error: !!loadError.value,
  status: loadError.value || (isRefreshing.value ? t('xcloudPage.refreshingHint') : isSearching.value ? resultCountLabel.value : cacheState.value === 'stale' && updatedAtLabel.value ? t('xcloudPage.cachedHint', { time: updatedAtLabel.value }) : resultCountLabel.value),
}))

function getVisibleSections(): HTMLElement[] {
  return XCLOUD_SECTION_IDS.map(id => document.querySelector<HTMLElement>(`[data-nav-section="${id}"]`)).filter((element): element is HTMLElement => element !== null)
}

function measureActiveSection(): void {
  contentFrame = 0
  if (!pageActive || !contentScroller)
    return
  const boundary = contentScroller.getBoundingClientRect().top + 24
  const sections = getVisibleSections()
  const last = sections.filter(section => section.getBoundingClientRect().top <= boundary).at(-1) ?? sections[0]
  if (last)
    activeSection.value = last.dataset.navSection ?? 'xcloud-all'
}
function handleContentScroll(): void {
  if (pageActive && !contentFrame)
    contentFrame = requestAnimationFrame(measureActiveSection)
}
function setupContentScroller(): void {
  contentScroller?.removeEventListener('scroll', handleContentScroll)
  contentScroller = contentRef.value
  if (pageActive)
    contentScroller?.addEventListener('scroll', handleContentScroll, { passive: true })
  handleContentScroll()
}
function saveBrowsePosition(): void {
  if (!contentRef.value?.isConnected)
    return
  browseScroll = contentRef.value.scrollTop
  pageRef.value?.querySelectorAll<HTMLElement>('[data-nav-section] .horizontal-list-rail__viewport').forEach((viewport) => {
    const section = viewport.closest<HTMLElement>('[data-nav-section]')?.dataset.navSection
    if (section)
      railScrolls.set(section, viewport.scrollLeft)
  })
}
function restoreBrowsePosition(): void {
  if (contentRef.value)
    contentRef.value.scrollTop = browseScroll
  pageRef.value?.querySelectorAll<HTMLElement>('[data-nav-section] .horizontal-list-rail__viewport').forEach((viewport) => {
    viewport.scrollLeft = railScrolls.get(viewport.closest<HTMLElement>('[data-nav-section]')?.dataset.navSection ?? '') ?? 0
  })
}
// KeepAlive 移出 DOM 前保存位置，重新挂载后恢复完整占位中的视口。
onBeforeRouteLeave(saveBrowsePosition)
async function jumpToSection(id: string): Promise<void> {
  const section = getVisibleSections().find(section => section.dataset.navSection === id)
  if (!section || !contentScroller || !pageActive)
    return
  const offset = section.getBoundingClientRect().top - contentScroller.getBoundingClientRect().top + contentScroller.scrollTop - 16
  contentScroller.scrollTo({ top: Math.max(0, offset), behavior: 'instant' })
  await nextTick()
  if (!pageActive)
    return
  await windows.get(id)?.activateAnchor()
  activeSection.value = id
  void prefs.update({ section: id })
}
watch(contentRef, setupContentScroller, { flush: 'post' })
// 搜索切换只变更窗口和滚动锚点，输入框即时显示输入值。
watch(isSearching, (searching) => {
  if (searching && contentScroller)
    searchScroll = contentScroller.scrollTop
}, { flush: 'sync' })
watch(isSearching, async (searching) => {
  await nextTick()
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
  if (pageActive && contentScroller && isSearching.value === searching) {
    contentScroller.scrollTop = searching ? 0 : searchScroll
    windows.forEach(window => window.measure())
  }
}, { flush: 'post' })

function handleResize(): void {
  if (pageActive)
    viewportWidth.value = window.innerWidth
}

function openDetail(title: XcloudTitle): void {
  anchorProduct = title.productId
  restoreCard = true
  pageFocus.save()
  void router.push({ name: 'cloud-game', params: { productId: title.productId } })
}
async function startStream(title: XcloudTitle): Promise<void> {
  if (starting.value)
    return
  starting.value = true
  startingProduct.value = title.productId
  loadError.value = ''
  const generation = catalogRequests.capture()
  try {
    const auth = await rpc.auth.getState()
    if (!pageActive || !catalogRequests.isCurrent(generation))
      return
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
    restoreCard = true
    pageFocus.save()
    await router.push({ name: 'xcloud-stream', params: { targetId: current.titleId }, query: { name: title.name } })
  }
  catch (cause) {
    if (catalogRequests.isCurrent(generation))
      loadError.value = cause instanceof Error ? cause.message : String(cause)
  }
  finally {
    starting.value = false
    startingProduct.value = ''
  }
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
  hasLoaded.value = hasLoaded.value || payload.cacheState !== 'miss' || !payload.refreshing
  loadError.value = payload.error ?? ''
  if (!restoredSection && !anchorProduct && prefs.preferences.value.section && payload.titles.length && !isSearching.value) {
    restoredSection = true
    void nextTick(() => {
      if (pageFocus.isActive() && !anchorProduct)
        jumpToSection(prefs.preferences.value.section)
    })
  }
  access.value = payload.access ?? 'browse'
  sources.value = payload.sources ?? {}
}

function clearCatalog(): void {
  stopComputing()
  catalogIndex.value = undefined
  indexedTitles = undefined
  gridSourceTitles.value = []
  catalogRequests.clear()
  wait.cancel()
  selectedProduct.value = ''
  titles.value = []
  cacheState.value = 'miss'
  updatedAt.value = undefined
  loadError.value = ''
  hasLoaded.value = false
  hasQueriedCatalog.value = false
  isLoading.value = false
  isRefreshing.value = false
  restoredSection = false
}

watch(locale, () => {
  catalog.clear()
  if (pageActive) {
    clearCatalog()
    void loadTitles()
  }
})

function loadTitles(forceRefresh = false): Promise<void> {
  if (!pageActive)
    return Promise.resolve()
  if (pendingLoad)
    return pendingLoad
  const operation = performLoad(forceRefresh).finally(() => {
    if (pendingLoad === operation)
      pendingLoad = undefined
  })
  pendingLoad = operation
  return operation
}
async function performLoad(forceRefresh: boolean): Promise<void> {
  const shouldBlock = !hasLoaded.value && titles.value.length === 0
  if (shouldBlock) {
    isLoading.value = true
  }
  else if (forceRefresh) {
    isRefreshing.value = true
  }

  const generation = catalogRequests.capture()
  // 可浏览目录先提交；账户与本地偏好各自补齐。
  void rpc.auth.getState().then((state) => {
    if (pageActive && catalogRequests.isCurrent(generation))
      appLevel.value = state.appLevel
  }).catch(() => {})
  void prefs.load().then(() => {
    if (pageActive && catalogRequests.isCurrent(generation)) {
      category.value = prefs.preferences.value.category
      sort.value = prefs.preferences.value.sort || 'name'
    }
  })
  try {
    await (forceRefresh ? catalog.refresh() : catalog.ensure())
  }
  catch (error) {
    if (!pageActive || !catalogRequests.isCurrent(generation))
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
    if (pageActive && catalogRequests.isCurrent(generation)) {
      isLoading.value = false
    }
  }
}

function activatePage() {
  if (pageActive)
    return
  pageActive = true
  handleResize()
  disposeCatalogUpdated = catalog.subscribe((payload) => {
    if (payload) {
      applyCatalogPayload(payload)
    }
    else {
      clearCatalog()
      pendingLoad = undefined
      void loadTitles()
    }
  })
  disposeAuthChanged = events.on('auth.stateChanged', (state) => {
    appLevel.value = state.appLevel
    if (!state.isAuthenticated) {
      void loadTitles()
    }
    // 静默续期保留在途目录请求，完成云授权后允许刷新。
    else if (!state.isAuthenticating && pageActive) {
      void loadTitles()
    }
  })
  void loadTitles()
  registerTabSwitch()
  void rebuildIndex()
  scheduleQuery(false)
  void nextTick(async () => {
    setupContentScroller()
    restoreBrowsePosition()
    windows.forEach(window => window.measure())
    if (restoreCard && anchorProduct && pageActive) {
      restoreCard = false
      await windows.get(anchorSection)?.reveal(anchorProduct, anchorAction)
    }
  })
}
onMounted(() => {
  window.addEventListener('resize', handleResize)
  disposeFocus = navigationEngine.onFocusChange((element) => {
    if (!pageFocus.isActive())
      return
    const id = element?.closest<HTMLElement>('[data-cloud-product]')?.dataset.cloudProduct
    if (id) {
      selectedProduct.value = id
      anchorProduct = id
      anchorSection = element?.closest<HTMLElement>('[data-nav-section]')?.dataset.navSection ?? 'xcloud-all'
      anchorAction = element?.id.endsWith('.play') ? 'play' : element?.id.endsWith('.favorite') ? 'favorite' : ''
      const title = selectedTitle.value
      wait.focus(id, access.value === 'account' && entitlement(title) === 'allowed')
    }
    else {
      wait.cancel()
    }
  })
  activatePage()
})

onActivated(activatePage)

onDeactivated(() => {
  saveBrowsePosition()
  pageActive = false
  stopComputing()
  catalogRequests.clear()
  pendingLoad = undefined
  disposeCatalogUpdated?.()
  disposeCatalogUpdated = undefined
  disposeAuthChanged?.()
  disposeAuthChanged = undefined
  wait.cancel()
  contentScroller?.removeEventListener('scroll', handleContentScroll)
  unregisterTabSwitch()
})

onBeforeUnmount(() => {
  pageActive = false
  stopComputing()
  contentScroller?.removeEventListener('scroll', handleContentScroll)
  window.removeEventListener('resize', handleResize)
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
            <p class="xcloud-page__subtitle" role="status">
              {{ access === 'account' ? t('xcloudUx.accountCatalog') : t('xcloudUx.browseCatalog') }} · {{ isRefreshing ? t('xcloudPage.refreshingHint') : updatedAtLabel ? t(cacheState === 'stale' ? 'xcloudPage.cachedHint' : 'xcloudPage.freshHint', { time: updatedAtLabel }) : t('xcloudUx.waitingCatalog') }}
              <span v-if="Object.values(sources).includes('unavailable')"> · {{ t('xcloudUx.partialSources') }}</span>
            </p>
          </div>
          <div class="xcloud-page__search-actions">
            <div class="xcloud-page__filters xcloud-page__jumps">
              <Focusable id="xcloud.filter.category" as="button" type="button" :aria-label="t('xcloudUx.category')" aria-haspopup="dialog" :title="category || t('xcloudUx.allCategories')" @click="filterSheet = 'category'">
                <span class="xcloud-page__filter-label">{{ category || t('xcloudUx.allCategories') }}</span>
                <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
              </Focusable>
              <Focusable id="xcloud.filter.sort" as="button" type="button" :aria-label="t('xcloudUx.sortLabel')" aria-haspopup="dialog" :title="t(`xcloudUx.sort.${sort}`)" @click="filterSheet = 'sort'">
                <span class="xcloud-page__filter-label">{{ t(`xcloudUx.sort.${sort}`) }}</span>
                <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
              </Focusable>
            </div>
            <div class="xcloud-page__search-controls">
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
        </div>
        <div v-if="isSearching || category" class="xcloud-page__jumps">
          <Focusable id="xcloud.clear" as="button" @click="searchKeyword = ''; category = ''; prefs.update({ category: '' })">
            {{ t('xcloudUx.clearFilters') }}
          </Focusable>
        </div>
        <nav v-if="!isSearching && (titles.length || rails.length)" class="xcloud-page__jumps xcloud-page__categories" data-nav-scroll-container :aria-label="t('shell.hints.regions')">
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
            <CloudCatalogWindow :ref="value => registerWindow(rail.id, value)" :titles="rail.titles" :section-id="rail.id" horizontal :favorite-ids="favoriteIds" :available-ids="availableIds" :starting-product="startingProduct" :wait-product="wait.product.value" :wait-meta="waitLabel()" @select="openDetail" @play="startStream" @favorite="prefs.favorite" @demand="prioritizeWindow(rail.id, $event)" />
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

          <div class="xcloud-page__grid">
            <CloudCatalogWindow :ref="value => registerWindow('xcloud-all', value)" :titles="gridSourceTitles" section-id="xcloud-all" :columns="gridColumnCount" :favorite-ids="favoriteIds" :available-ids="availableIds" :starting-product="startingProduct" :wait-product="wait.product.value" :wait-meta="waitLabel()" @select="openDetail" @play="startStream" @favorite="prefs.favorite" @demand="prioritizeWindow('xcloud-all', $event)" />
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
.xcloud-page__categories { flex-wrap: nowrap; min-width: 0; overflow-x: auto; overflow-y: hidden; margin: calc(-1 * var(--ui-focus-safe-inset)); padding: var(--ui-focus-safe-inset); scroll-padding: var(--ui-focus-safe-inset); }
.xcloud-page__categories button { flex: 0 0 auto; white-space: nowrap; }

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

.xcloud-page__search-actions { display: flex; align-items: center; gap: var(--space-12); flex: 0 1 580px; min-width: 0; flex-wrap: wrap; }
.xcloud-page__search-controls { display: flex; align-items: center; gap: var(--space-12); flex: 1 1 248px; min-width: 0; }
.xcloud-page__filters { flex: 0 1 auto; min-width: 0; flex-wrap: nowrap; }
.xcloud-page__filters button { display: flex; align-items: center; gap: var(--space-8); min-width: 0; min-height: 36px; }
.xcloud-page__filters svg { flex: 0 0 14px; width: 14px; height: 14px; }
.xcloud-page__filter-label { max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
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
  flex: 1 1 200px;
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
  flex: 0 0 auto;
}

.xcloud-page__grid-section {
  display: flex;
  flex: 0 0 auto;
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
  padding: var(--space-12) var(--space-8);
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

@media(max-width:900px) { .xcloud-page__toolbar { flex-direction: column; align-items: stretch; gap: var(--space-16); } .xcloud-page__search-actions { flex-basis: auto; width: 100%; } }
@media(max-width:700px) { .xcloud-page__title { font-size: 24px; } .xcloud-page__results { white-space: normal; } .xcloud-page__jumps { gap: var(--space-4); } }
</style>
