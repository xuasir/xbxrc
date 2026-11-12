<script setup lang="ts">
import type { CloudDetail, CloudTitle } from '@/shared/rpc/cloud'
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { onBeforeRouteLeave, useRoute, useRouter } from 'vue-router'
import HorizontalListRail from '@/components/common/HorizontalListRail.vue'
import RemoteImage from '@/components/common/RemoteImage.vue'
import SettingModalShell from '@/components/settings/SettingModalShell.vue'
import { usePageFocus } from '@/navigation/core/usePageFocus'
import { Focusable } from '@/navigation/core/vue'
import { events } from '@/services/events'
import { rpc } from '@/services/rpc'
import { formatMinutes } from './library/library-view'
import { useLibraryQuery } from './library/use-library-query'
import { entitlement } from './xcloud/catalog-model'
import { cloudDetailView, saveCloudDetailView } from './xcloud/detail-view-state'
import { useCloudCatalog } from './xcloud/use-cloud-catalog'
import { useCloudPreferences } from './xcloud/use-cloud-preferences'
import { useCloudWait } from './xcloud/use-cloud-wait'
import './library/library.scss'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const root = ref<HTMLElement | null>(null)
const productId = ref('')
const mediaWidth = ref(typeof window === 'undefined' ? 1280 : window.innerWidth)
const catalog = useCloudCatalog()
let active = false
const activeView = ref(false)
let disposeCatalog: (() => void) | undefined
let disposeAuth: (() => void) | undefined
let viewKey = ''
let summaryPending: { current: number, id: string, operation: Promise<void> } | undefined
const prefs = useCloudPreferences()
const wait = useCloudWait()
const detail = shallowRef<CloudDetail>()
const title = shallowRef<CloudTitle>()
const loading = ref(false)
const starting = ref(false)
const error = ref('')
const accessError = ref('')
const expandedDescription = ref(false)
const allLanguages = ref(false)
const previewIndex = ref(-1)
const screenshots = computed(() => detail.value?.detail.screenshots ?? [])
const preview = computed(() => screenshots.value[previewIndex.value])
let generation = 0
usePageFocus(() => root.value, () => productId.value, () => 'cloud.detail.play')
const favorite = computed(() => prefs.preferences.value.favorites.some(item => item.productId === productId.value))
const pinned = computed(() => prefs.preferences.value.pinned.includes(productId.value))
const name = computed(() => detail.value?.detail.product.name || title.value?.name || productId.value)
const summary = computed<CloudTitle | undefined>(() => {
  if (title.value)
    return title.value
  const product = detail.value?.detail.product
  if (!product)
    return undefined
  return { ...product, id: product.productId, titleId: '', aliases: [], supportedInputTypes: [], hasEntitlement: false, entitlementKnown: false, isRecentlyPlayed: false, isNew: false }
})
const description = computed(() => detail.value?.detail.product.description || summary.value?.description || '')
const titleId = computed(() => title.value?.xboxTitleId ? String(title.value.xboxTitleId) : '')
const history = useLibraryQuery(() => activeView.value && route.name === 'cloud-game' && String(route.params.productId ?? '') === productId.value && titleId.value ? `cloud-game:${titleId.value}` : '', () => rpc.data.getLibraryGame({ xboxTitleId: titleId.value }))
const times = useLibraryQuery(() => activeView.value && route.name === 'cloud-game' && String(route.params.productId ?? '') === productId.value && titleId.value ? `cloud-time:${titleId.value}` : '', () => rpc.data.getPlaytimes({ xboxTitleIds: [titleId.value] }))
const progress = computed(() => history.state.value.value?.progress)
const languages = computed(() => {
  const entries = [...(detail.value?.detail.languages ?? [])]
  const preferred = locale.value.startsWith('zh') ? ['zh-CN', 'zh-TW', 'en-US'] : ['en-US', 'en-GB']
  entries.sort((a, b) => (preferred.includes(a.locale) ? preferred.indexOf(a.locale) : 99) - (preferred.includes(b.locale) ? preferred.indexOf(b.locale) : 99) || a.locale.localeCompare(b.locale))
  return allLanguages.value ? entries : entries.slice(0, 4)
})
function languageName(value: string) {
  try {
    return new Intl.DisplayNames([locale.value], { type: 'language' }).of(value) || value
  }
  catch {
    return value
  }
}
function personal(tab = 'overview') {
  void router.push({ name: 'my-game', params: { xboxTitleId: titleId.value }, query: { sourceProductId: productId.value, tab } })
}
function changePreview(delta: number) {
  previewIndex.value = (previewIndex.value + delta + screenshots.value.length) % screenshots.value.length
}
function updateMediaSize() {
  if (active && typeof window !== 'undefined')
    mediaWidth.value = root.value?.clientWidth || window.innerWidth
}
function saveView() {
  if (root.value?.isConnected === false)
    return
  if (viewKey)
    saveCloudDetailView(viewKey, { detail: detail.value, expanded: expandedDescription.value, languages: allLanguages.value, scrollTop: root.value?.querySelector('.cloud-detail__scroll')?.scrollTop ?? 0, screenshotScroll: root.value?.querySelector('.cloud-detail__screenshots .horizontal-list-rail__viewport')?.scrollLeft ?? 0 })
}
onBeforeRouteLeave(saveView)
function restoreView(key: string) {
  const saved = cloudDetailView(key)
  void nextTick(() => {
    if (!active || viewKey !== key)
      return
    const scroll = root.value?.querySelector('.cloud-detail__scroll')
    if (scroll)
      scroll.scrollTop = saved?.scrollTop ?? 0
    const screenshots = root.value?.querySelector('.cloud-detail__screenshots .horizontal-list-rail__viewport')
    if (screenshots)
      screenshots.scrollLeft = saved?.screenshotScroll ?? 0
    updateMediaSize()
  })
}
function isCurrent(current: number, id: string): boolean {
  return active && current === generation && route.name === 'cloud-game' && String(route.params.productId ?? '').trim() === id && productId.value === id
}
function synchronizeProduct() {
  if (!active || route.name !== 'cloud-game')
    return
  const id = String(route.params.productId ?? '').trim()
  if (!id)
    return
  const key = `${locale.value}:${id}`
  if (key !== viewKey) {
    saveView()
    generation++
    loading.value = false
    starting.value = false
    wait.cancel()
    viewKey = key
    productId.value = id
    const saved = cloudDetailView(key)
    detail.value = saved?.detail
    title.value = catalog.getTitle(id)
    expandedDescription.value = saved?.expanded ?? false
    allLanguages.value = saved?.languages ?? false
    previewIndex.value = -1
    restoreView(key)
    void load()
  }
}
function activate() {
  if (active)
    return
  active = true
  activeView.value = true
  if (typeof window !== 'undefined')
    window.addEventListener('resize', updateMediaSize)
  void nextTick(updateMediaSize)
  disposeCatalog = catalog.subscribe((value) => {
    if (!active || !productId.value)
      return
    title.value = value ? catalog.getTitle(productId.value) : undefined
  })
  disposeAuth = events.on('auth.stateChanged', (state) => {
    if (!state.isAuthenticated) {
      generation++
      loading.value = false
      title.value = undefined
      wait.cancel()
    }
    if (!state.isAuthenticating && active && productId.value)
      void loadSummary(generation, productId.value)
  })
  const previous = viewKey
  synchronizeProduct()
  if (previous && previous === viewKey) {
    restoreView(viewKey)
    void load()
  }
}
function deactivate() {
  saveView()
  active = false
  activeView.value = false
  if (typeof window !== 'undefined')
    window.removeEventListener('resize', updateMediaSize)
  summaryPending = undefined
  previewIndex.value = -1
  generation++
  loading.value = false
  starting.value = false
  disposeCatalog?.()
  disposeCatalog = undefined
  disposeAuth?.()
  disposeAuth = undefined
  wait.cancel()
}
function loadSummary(current: number, id: string): Promise<void> {
  if (!isCurrent(current, id))
    return Promise.resolve()
  if (summaryPending?.current === current && summaryPending.id === id)
    return summaryPending.operation
  const operation = catalog.summary(id).then((value) => {
    if (isCurrent(current, id))
      title.value = value
  }).catch(() => {}).finally(() => {
    if (summaryPending?.operation === operation)
      summaryPending = undefined
  })
  summaryPending = { current, id, operation }
  return operation
}
async function load() {
  if (!active || route.name !== 'cloud-game' || !productId.value || loading.value)
    return
  const current = ++generation
  const id = productId.value
  loading.value = true
  error.value = ''
  accessError.value = ''
  void prefs.load()
  // 商品资料与访问事实分别更新，访问失败仍可阅读详情。
  void loadSummary(current, id)
  try {
    const value = await rpc.data.getCloudDetail({ productId: id })
    if (isCurrent(current, id))
      detail.value = value
  }
  catch (cause) {
    if (isCurrent(current, id))
      error.value = cause instanceof Error ? cause.message : String(cause)
  }
  finally {
    if (isCurrent(current, id))
      loading.value = false
  }
}
async function play() {
  if (starting.value)
    return
  const current = generation
  const id = productId.value
  starting.value = true
  accessError.value = ''
  try {
    const auth = await rpc.auth.getState()
    if (!isCurrent(current, id))
      return
    if (!auth.isAuthenticated) {
      await router.push({ name: 'login' })
      return
    }
    const value = await rpc.data.resolveCloudTitle({ productId: id })
    if (current !== generation)
      return
    title.value = value
    if (entitlement(value) !== 'allowed') {
      accessError.value = t(`xcloudUx.${entitlement(value)}`)
      return
    }
    await router.push({ name: 'xcloud-stream', params: { targetId: value.titleId }, query: { name: name.value } })
  }
  catch (cause) {
    if (current === generation)
      accessError.value = cause instanceof Error ? cause.message : String(cause)
  }
  finally {
    if (current === generation)
      starting.value = false
  }
}
function back() {
  void router.push({ name: 'xcloud' })
}
function navBack(event: Event) {
  event.preventDefault()
  event.stopPropagation()
  back()
}
function waitLabel() {
  if (wait.loading.value)
    return t('xcloudUx.waitLoading')
  if (wait.error.value)
    return t('xcloudUx.waitFailed')
  if (wait.seconds.value == null)
    return t('xcloudUx.waitUnknown')
  return t(wait.seconds.value === 0 ? 'xcloudUx.waitReady' : 'xcloudUx.waitSeconds', { seconds: wait.seconds.value })
}
function retryWait() {
  wait.cancel()
  wait.focus(productId.value, entitlement(title.value) === 'allowed')
}
function languageFlag(value: number | null) {
  return t(value === 1 ? 'xcloudUx.support' : value === 2 ? 'xcloudUx.unsupported' : 'xcloudUx.unspecified')
}
watch(() => [route.name, route.params.productId, locale.value], synchronizeProduct, { flush: 'post' })
watch(title, (value) => {
  if (active)
    wait.focus(productId.value, entitlement(value) === 'allowed')
})
onMounted(activate)
onActivated(activate)
onDeactivated(deactivate)
onBeforeUnmount(deactivate)
</script>

<template>
  <section ref="root" class="cloud-detail ui-page-shell" @nav-back="navBack">
    <header class="cloud-detail__toolbar">
      <Focusable id="cloud.detail.back" as="button" class="cloud-detail__back" @click="back">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m14 6-6 6 6 6" /></svg>{{ t('xcloudUx.back') }}
      </Focusable>
      <div class="cloud-detail__tools">
        <Focusable id="cloud.detail.favorite" as="button" class="cloud-detail__tool" :class="{ 'is-selected': favorite }" :disabled="!summary" :aria-pressed="favorite" :aria-label="t(favorite ? 'xcloudUx.unfavorite' : 'xcloudUx.favorite')" :title="t(favorite ? 'xcloudUx.unfavorite' : 'xcloudUx.favorite')" @click="summary && prefs.favorite(summary)">
          <svg viewBox="0 0 24 24" :fill="favorite ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="M20.5 5.5a5 5 0 0 0-7 0L12 7l-1.5-1.5a5 5 0 0 0-7 7L12 21l8.5-8.5a5 5 0 0 0 0-7Z" /></svg>
        </Focusable>
        <Focusable v-if="favorite" id="cloud.detail.pin" as="button" class="cloud-detail__tool" :class="{ 'is-selected': pinned }" :aria-pressed="pinned" :aria-label="t(pinned ? 'xcloudUx.unpin' : 'xcloudUx.pin')" :title="t(pinned ? 'xcloudUx.unpin' : 'xcloudUx.pin')" @click="prefs.pin(productId)">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" aria-hidden="true"><path d="m8 3 8 0-1 7 4 4H5l4-4-1-7ZM12 14v7" /></svg>
        </Focusable>
        <Focusable id="cloud.detail.refresh" as="button" class="cloud-detail__tool" :busy="loading" :aria-label="t('xcloudUx.retry')" :title="t('xcloudUx.retry')" @click="load">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5M6.2 6.2A8 8 0 0 1 20 12M4 12a8 8 0 0 0 13.8 5.8" /></svg>
        </Focusable>
      </div>
    </header>
    <div class="cloud-detail__scroll" data-nav-scroll-container>
      <p v-if="loading" class="cloud-detail__notice" role="status">
        {{ t('xcloudUx.loading') }}
      </p>
      <p v-if="detail?.stale" class="cloud-detail__notice" role="status">
        {{ t('xcloudUx.cachedDetail') }} · {{ new Date(detail.updatedAt).toLocaleString() }}
      </p>
      <p v-if="error || accessError || prefs.error.value" class="cloud-detail__notice cloud-detail__notice--error" role="alert">
        {{ error || accessError || prefs.error.value }}
      </p>
      <section class="cloud-detail__hero" :aria-label="name">
        <RemoteImage class="cloud-detail__hero-image" :size="mediaWidth" :src="detail?.detail.product.heroImageUrl" :sources="[detail?.detail.product.posterImageUrl ?? '', summary?.tileImageUrl ?? '']" />
        <div class="cloud-detail__hero-shade" />
        <div class="cloud-detail__hero-copy">
          <p class="cloud-detail__eyebrow">
            {{ t('xcloudUx.cloudLabel') }}
          </p>
          <h1>{{ name }}</h1>
          <p class="cloud-detail__publisher">
            {{ detail?.detail.product.publisherName || summary?.publisherName }}
          </p>
          <div class="cloud-detail__chips">
            <span v-for="category in detail?.detail.product.categories" :key="category">{{ category }}</span>
            <span v-if="detail?.detail.rating">{{ detail.detail.rating }}</span>
          </div>
          <div class="cloud-detail__hero-actions">
            <Focusable id="cloud.detail.play" as="button" class="cloud-detail__play" :busy="starting" :disabled="entitlement(title) === 'denied'" @click="play">
              <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M8 5.5a1 1 0 0 1 1.5-.86l10 6.5a1 1 0 0 1 0 1.72l-10 6.5A1 1 0 0 1 8 18.5z" /></svg>{{ t('xcloudUx.play') }}
            </Focusable>
            <div class="cloud-detail__ready">
              <span class="cloud-detail__dot" :class="{ 'is-ready': entitlement(title) === 'allowed' }" /><span>{{ entitlement(title) === 'allowed' ? waitLabel() : t(`xcloudUx.${entitlement(title)}`) }}</span>
            </div>
            <Focusable v-if="wait.error.value" id="cloud.detail.waitRetry" as="button" class="cloud-detail__text-button cloud-detail__text-button--on-media" @click="retryWait">
              {{ t('xcloudUx.retry') }}
            </Focusable>
          </div>
        </div>
      </section>
      <div class="cloud-detail__body">
        <div class="cloud-detail__main">
          <section class="cloud-detail__section">
            <div class="cloud-detail__section-heading">
              <h2>{{ t('xcloudUx.about') }}</h2>
            </div>
            <p class="cloud-detail__description" :class="{ 'is-collapsed': !expandedDescription }">
              {{ description || t('xcloudUx.loading') }}
            </p>
            <Focusable v-if="description.length > 260" id="cloud.detail.description" as="button" class="cloud-detail__text-button" :aria-expanded="expandedDescription" @click="expandedDescription = !expandedDescription">
              {{ t(expandedDescription ? 'xcloudUx.showLess' : 'xcloudUx.showMore') }}
            </Focusable>
            <div v-if="detail?.detail.attributes.length" class="cloud-detail__features">
              <span v-for="attribute in detail.detail.attributes" :key="attribute">{{ attribute }}</span>
            </div>
          </section>
          <section v-if="screenshots.length" class="cloud-detail__section">
            <HorizontalListRail class="cloud-detail__screenshots" :title="t('xcloudUx.screenshots')" :hint="String(screenshots.length)" :aria-label="t('xcloudUx.screenshots')">
              <Focusable v-for="(src, index) in screenshots" :id="`cloud.detail.screen.${index}`" :key="src" as="button" class="cloud-detail__screenshot" :aria-label="`${t('xcloudUx.screenshots')} ${index + 1}`" @click="previewIndex = index">
                <RemoteImage :src="src" :size="360" /><span class="cloud-detail__expand-icon" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 4H4v5M15 4h5v5M4 15v5h5M20 15v5h-5" /></svg></span>
              </Focusable>
            </HorizontalListRail>
          </section>
          <section class="cloud-detail__section cloud-detail__languages">
            <div class="cloud-detail__section-heading">
              <h2>{{ t('xcloudUx.languages') }}</h2>
            </div>
            <p v-if="!languages.length" class="cloud-detail__muted">
              {{ t('xcloudUx.languageUnknown') }}
            </p>
            <table v-else>
              <thead><tr><th>{{ t('xcloudUx.language') }}</th><th>{{ t('xcloudUx.interface') }}</th><th>{{ t('xcloudUx.audio') }}</th><th>{{ t('xcloudUx.subtitles') }}</th></tr></thead><tbody>
                <tr v-for="language in languages" :key="language.locale">
                  <th>{{ languageName(language.locale) }}</th><td v-for="(flag, key) in { interface: language.interface, audio: language.audio, subtitles: language.subtitles }" :key="key" :class="{ 'is-supported': flag === 1 }">
                    <span v-if="flag === 1" aria-hidden="true">✓ </span>{{ languageFlag(flag) }}
                  </td>
                </tr>
              </tbody>
            </table>
            <Focusable v-if="(detail?.detail.languages.length ?? 0) > 4" id="cloud.detail.languages" as="button" class="cloud-detail__text-button" :aria-expanded="allLanguages" @click="allLanguages = !allLanguages">
              {{ allLanguages ? t('xcloudUx.showLess') : t('xcloudUx.allLanguages', { count: detail?.detail.languages.length }) }}
            </Focusable>
          </section>
          <section v-if="detail?.detail.accessibility.length" class="cloud-detail__section">
            <h2>{{ t('xcloudUx.accessibility') }}</h2><div class="cloud-detail__features">
              <span v-for="feature in detail.detail.accessibility" :key="feature">{{ feature }}</span>
            </div>
          </section>
        </div>
        <aside class="cloud-detail__aside">
          <section class="cloud-detail__info-card">
            <p class="cloud-detail__eyebrow cloud-detail__eyebrow--muted">
              {{ t('xcloudUx.gameInfo') }}
            </p>
            <dl>
              <template v-for="fact in [{ label: t('xcloudUx.publisher'), value: detail?.detail.product.publisherName }, { label: t('xcloudUx.developer'), value: detail?.detail.developer }, { label: t('xcloudUx.releaseLabel'), value: detail?.detail.releaseDate ? new Date(detail.detail.releaseDate).toLocaleDateString() : '' }, { label: t('xcloudUx.ratingLabel'), value: detail?.detail.rating }]" :key="fact.label">
                <div v-if="fact.value">
                  <dt>{{ fact.label }}</dt><dd>{{ fact.value }}</dd>
                </div>
              </template>
            </dl>
            <div class="cloud-detail__platforms">
              <span v-for="platform in detail?.detail.platforms" :key="platform">{{ platform }}</span>
            </div>
          </section>
          <section v-if="titleId" class="cloud-detail__info-card">
            <p class="cloud-detail__eyebrow cloud-detail__eyebrow--muted">
              {{ t('xcloudUx.yourProgress') }}
            </p>
            <div class="cloud-detail__records">
              <div><strong>{{ formatMinutes(times.state.value.value?.[0]?.minutes) }}</strong><span>{{ t('xcloudUx.playtime') }}</span></div><div v-if="progress">
                <strong>{{ progress.unlockedCount }} <small>/ {{ progress.totalCount }}</small></strong><span>{{ t('xcloudUx.achievements') }}</span>
              </div>
            </div>
            <p v-if="times.state.value.error || history.state.value.error" class="cloud-detail__muted">
              {{ times.state.value.error || history.state.value.error }}
            </p>
            <div class="cloud-detail__record-actions">
              <Focusable id="cloud.detail.achievements" as="button" class="cloud-detail__text-button" @click="personal('achievements')">
                {{ t('xcloudUx.achievements') }} ↗
              </Focusable><Focusable id="cloud.detail.album" as="button" class="cloud-detail__text-button" @click="personal('album')">
                {{ t('xcloudUx.album') }} ↗
              </Focusable>
            </div>
          </section>
          <section class="cloud-detail__ready-card">
            <span class="cloud-detail__controller" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"><path d="M7.5 7h9c2 0 3 1.5 3.5 3.5l1 5c.5 2.5-2 4-3.5 2.5L15 15H9l-2.5 3c-1.5 1.5-4 0-3.5-2.5l1-5C4.5 8.5 5.5 7 7.5 7Z" /><path d="M6 11h4M8 9v4M16 10h.01M18 12h.01" /></svg></span><div><strong>{{ t('xcloudUx.controllerInput') }}</strong><p>{{ t('xcloudUx.heroHint') }}</p></div>
          </section>
        </aside>
      </div>
    </div>
    <SettingModalShell :open="previewIndex >= 0" scope-id="cloud.detail.image" :title="t('xcloudUx.screenshots')" :eyebrow="name" width="min(100%, 1120px)" @close="previewIndex = -1">
      <RemoteImage class="cloud-detail__preview" :size="mediaWidth" :src="preview" :alt="`${name} ${previewIndex + 1}`" />
      <div class="cloud-detail__preview-actions">
        <Focusable id="cloud.detail.image.previous" as="button" :aria-label="t('xcloudUx.previousImage')" class="ui-action-button" @click="changePreview(-1)">
          ←
        </Focusable><span>{{ previewIndex + 1 }} / {{ screenshots.length }}</span><Focusable id="cloud.detail.image.next" as="button" :aria-label="t('xcloudUx.nextImage')" class="ui-action-button" @click="changePreview(1)">
          →
        </Focusable>
      </div>
    </SettingModalShell>
  </section>
</template>

<style scoped>
.cloud-detail { display: flex; flex-direction: column; height: 100%; min-height: 0; overflow: hidden; gap: 0; }
.cloud-detail__toolbar { display: flex; flex: 0 0 auto; justify-content: space-between; align-items: center; min-height: 52px; gap: 16px; padding: 4px var(--ui-focus-safe-inset) 12px; }
.cloud-detail__back { display: flex; gap: 6px; align-items: center; border: 0; background: transparent; color: var(--color-text-secondary); font: inherit; font-size: 13px; padding: 8px 4px; border-radius: var(--ui-radius-md); cursor: pointer; }
.cloud-detail__back svg { width: 20px; height: 20px; }
.cloud-detail__tools { display: flex; gap: 8px; }
.cloud-detail__tool { width: 36px; height: 36px; display: grid; place-items: center; padding: 8px; border: 1px solid var(--color-border-subtle); border-radius: 50%; background: transparent; color: var(--color-text-secondary); cursor: pointer; }
.cloud-detail__tool svg { width: 100%; height: 100%; }
.cloud-detail__tool.is-selected { color: var(--color-text-primary); background: var(--ui-choice-selected-bg); border-color: var(--ui-choice-selected-border); }
.cloud-detail__scroll { flex: 1; min-height: 0; overflow: auto; padding: var(--ui-focus-safe-inset); scroll-padding: var(--ui-focus-safe-inset); }
.cloud-detail__hero { position: relative; overflow: hidden; isolation: isolate; min-height: clamp(300px,32vw,430px); display: flex; align-items: flex-end; border-radius: 16px; background: #111f24; border: 1px solid rgba(255,255,255,.08); }
.cloud-detail__hero-image, .cloud-detail__hero-shade { position: absolute; inset: 0; z-index: -1; }
.cloud-detail__hero-image { width: 100%; height: 100%; object-fit: cover; }
.cloud-detail__hero-shade { background: linear-gradient(90deg,rgba(4,12,17,.88),rgba(4,12,17,.35) 70%,rgba(4,12,17,.2)),linear-gradient(0deg,rgba(4,12,17,.9),transparent 80%); }
.cloud-detail__hero-copy { width: 100%; padding: 32px 36px; color: #fff; }
.cloud-detail__eyebrow { font-size: 10px; line-height: 1.5; letter-spacing: .16em; font-weight: 600; color: #b1d6ba; margin: 0 0 12px; }
.cloud-detail h1 { font-size: clamp(30px,3.8vw,52px); line-height: 1.08; font-weight: 600; letter-spacing: -.035em; max-width: 740px; margin: 0 0 12px; text-wrap: balance; }
.cloud-detail__publisher { color: rgba(255,255,255,.7); font-size: 13px; line-height: 1.5; margin: 0 0 14px; }
.cloud-detail__chips { display: flex; gap: 7px; flex-wrap: wrap; }
.cloud-detail__chips span { font-size: 10px; line-height: 1.5; color: rgba(255,255,255,.8); padding: 3px 8px; border: 1px solid rgba(255,255,255,.2); border-radius: 5px; background: rgba(255,255,255,.06); }
.cloud-detail__hero-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 16px; margin-top: 24px; }
.cloud-detail__play { display: flex; align-items: center; justify-content: center; gap: 9px; min-width: 156px; min-height: 46px; padding: 10px 22px; font: inherit; font-size: 14px; font-weight: 500; border: 1px solid rgba(197,255,188,.22); border-radius: 8px; color: #fff; background: #287a35; cursor: pointer; }
.cloud-detail__play svg { width: 18px; height: 18px; }
.cloud-detail__play:hover { background: #328a40; }
.cloud-detail__play:disabled { opacity: .45; cursor: default; }
.cloud-detail__ready { display: flex; align-items: center; gap: 8px; font-size: 11px; line-height: 1.5; color: rgba(255,255,255,.7); }
.cloud-detail__dot { width: 5px; height: 5px; border-radius: 50%; background: #d2bb86; flex-shrink: 0; }
.cloud-detail__dot.is-ready { background: #8dcc93; }
.cloud-detail__body { display: grid; grid-template-columns: minmax(0,1fr) minmax(240px,29%); gap: 32px; padding: 30px 6px; }
.cloud-detail__main { min-width: 0; display: flex; flex-direction: column; gap: 30px; }
.cloud-detail h2 { font-size: 17px; font-weight: 500; line-height: 1.4; color: var(--color-text-primary); margin: 0; letter-spacing: -.015em; }
.cloud-detail__section-heading { display: flex; align-items: center; gap: 10px; margin-bottom: 15px; }
.cloud-detail__section-heading > span { color: var(--color-text-tertiary); font-size: 12px; }
.cloud-detail__description { font-size: 13px; line-height: 1.85; color: var(--color-text-secondary); white-space: pre-line; margin: 0; }
.cloud-detail__description.is-collapsed { display: -webkit-box; -webkit-line-clamp: 5; -webkit-box-orient: vertical; overflow: hidden; }
.cloud-detail__text-button { border: 0; background: transparent; color: var(--color-text-primary); font: inherit; font-size: 12px; line-height: 1.5; padding: 6px 1px; border-radius: 4px; cursor: pointer; text-align: left; }
.cloud-detail__text-button--on-media { color: #fff; }
.cloud-detail__features { display: flex; flex-wrap: wrap; gap: 7px; margin-top: 15px; }
.cloud-detail__features span, .cloud-detail__platforms span { color: var(--color-text-secondary); font-size: 10px; line-height: 1.5; background: var(--ui-material-selected); border: 1px solid var(--color-border-subtle); padding: 5px 8px; border-radius: 5px; }
.cloud-detail__screenshots { min-width: 0; }
.cloud-detail__screenshots :deep(.horizontal-list-rail__title) { font-size: 18px; }
.cloud-detail__screenshot { position: relative; flex: 0 0 clamp(220px, 45vw, 360px); width: clamp(220px, 45vw, 360px); padding: 0; border: 1px solid var(--color-border-subtle); border-radius: 9px; overflow: hidden; background: transparent; cursor: pointer; }
.cloud-detail__screenshot .remote-image { aspect-ratio: 16/9; width: 100%; object-fit: cover; }
.cloud-detail__expand-icon { position: absolute; right: 8px; bottom: 8px; width: 26px; height: 26px; padding: 5px; border-radius: 6px; background: rgba(0,0,0,.5); color: #fff; }
.cloud-detail__expand-icon svg { width: 100%; height: 100%; }
.cloud-detail__languages table { width: 100%; border-collapse: collapse; font-size: 11px; line-height: 1.6; }
.cloud-detail__languages th { text-align: left; font-weight: 400; }
.cloud-detail__languages thead th { color: var(--color-text-tertiary); font-size: 10px; }
.cloud-detail__languages th, .cloud-detail__languages td { padding: 10px 8px; border-bottom: 1px solid var(--color-border-subtle); color: var(--color-text-secondary); }
.cloud-detail__languages .is-supported { color: var(--color-text-primary); }
.cloud-detail__aside { display: flex; flex-direction: column; gap: 16px; align-self: start; }
.cloud-detail__info-card { padding: 20px; border: 1px solid var(--color-border-subtle); border-radius: 11px; background: linear-gradient(135deg,var(--ui-panel-top),var(--ui-panel-bottom)); }
.cloud-detail__eyebrow--muted { color: var(--color-text-tertiary); letter-spacing: .08em; margin-bottom: 18px; }
.cloud-detail__info-card dl { display: flex; flex-direction: column; gap: 15px; margin: 0; }
.cloud-detail__info-card dt { font-size: 10px; color: var(--color-text-tertiary); margin-bottom: 4px; }
.cloud-detail__info-card dd { margin: 0; font-size: 12px; line-height: 1.5; color: var(--color-text-primary); }
.cloud-detail__platforms { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 20px; }
.cloud-detail__records { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.cloud-detail__records > div { display: flex; flex-direction: column; gap: 8px; }
.cloud-detail__records strong { font-size: 19px; font-weight: 500; color: var(--color-text-primary); line-height: 1.3; }
.cloud-detail__records small { font-size: 12px; color: var(--color-text-tertiary); }
.cloud-detail__records span { font-size: 10px; color: var(--color-text-tertiary); }
.cloud-detail__record-actions { display: flex; justify-content: space-between; border-top: 1px solid var(--color-border-subtle); margin-top: 16px; padding-top: 8px; }
.cloud-detail__ready-card { display: flex; gap: 12px; padding: 8px 3px; align-items: center; color: var(--color-text-secondary); }
.cloud-detail__controller { flex: 0 0 28px; width: 28px; height: 28px; }
.cloud-detail__controller svg { width: 100%; height: 100%; }
.cloud-detail__ready-card strong { font-size: 11px; font-weight: 400; }
.cloud-detail__ready-card p { margin: 4px 0 0; font-size: 10px; color: var(--color-text-tertiary); }
.cloud-detail__notice { margin: 0 0 10px; padding: 8px 12px; border-radius: 6px; font-size: 12px; line-height: 1.5; color: var(--color-text-secondary); background: var(--ui-material-selected); }
.cloud-detail__notice--error { color: var(--color-text-primary); }
.cloud-detail__muted { font-size: 11px; line-height: 1.5; color: var(--color-text-tertiary); }
.cloud-detail__preview { aspect-ratio: 16/9; width: 100%; object-fit: contain; }
.cloud-detail__preview-actions { display: flex; gap: 16px; align-items: center; justify-content: center; padding-top: 12px; font-size: 12px; color: var(--color-text-secondary); }
.cloud-detail__preview-actions button { min-width: 44px; }
.cloud-detail button[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.cloud-detail button:hover { filter: brightness(1.1); }
@media(max-width:850px) { .cloud-detail__body { grid-template-columns: 1fr; gap: 28px; padding: 24px 2px; } .cloud-detail__aside { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); } }
@media(max-width:540px) { .cloud-detail__hero { min-height: 330px; border-radius: 12px; } .cloud-detail__hero-copy { padding: 24px 20px; } .cloud-detail__hero-actions { gap: 12px; } .cloud-detail__aside { grid-template-columns: 1fr; } .cloud-detail__screenshots { gap: 8px; } .cloud-detail__languages th, .cloud-detail__languages td { padding: 9px 5px; font-size: 10px; } .cloud-detail__records strong { font-size: 20px; } }
</style>
