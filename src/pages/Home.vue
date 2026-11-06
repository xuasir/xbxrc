<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { usePageFocus } from '@/navigation/core/usePageFocus'
import { Focusable } from '@/navigation/core/vue'
import seriesSImage from '../assets/console/series-s-transparent.png'
import seriesXImage from '../assets/console/series-x-transparent.png'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import ConsoleStatusCard from '../components/common/ConsoleStatusCard.vue'
import { useShellPageHints } from '../components/layout/shell-context'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'
import { devWarn } from '../shared/dev-log'
import LibraryRefresh from './library/LibraryRefresh.vue'

const pageRef = ref<HTMLElement | null>(null)
const { t } = useI18n()
const router = useRouter()

type HostSummary = Awaited<ReturnType<typeof rpc.data.getHosts>>[number]

interface HostCardViewModel {
  host: HostSummary
  sourceIndex: number
  consoleType: string
  hostKey: string
  nodeId: string
  title: string
  imageSrc: string
  status: string
  description: string
}

const isLoading = ref(false)
const hosts = ref<HostSummary[]>([])
const selectedHostKey = ref('')
const loadError = ref('')

function resolveHostId(host: HostSummary, index: number): string {
  return host.serverId ?? host.id ?? host.deviceId ?? `host-${index}`
}

function resolveStreamTargetId(host: HostSummary, index: number): string {
  return host.serverId ?? host.id ?? host.deviceId ?? `host-${index}`
}

function resolveConsoleCommandId(host: HostSummary, index: number): string {
  // XCCS RemoteManagement 命令优先使用 console identity（`id`），
  // 而 xHome 建 session 仍应继续优先走 `serverId`。
  return host.id ?? host.serverId ?? host.deviceId ?? `host-${index}`
}

function resolveHostTitle(host: HostSummary): string {
  const title = host.name ?? host.deviceName
  return typeof title === 'string' && title.trim() !== '' ? title.trim() : 'Xbox'
}

function resolveHostConsoleType(host: HostSummary): string {
  return typeof host.consoleType === 'string' && host.consoleType !== '' ? host.consoleType : 'Xbox'
}

function resolveConsoleImage(consoleType: string): string {
  if (consoleType === 'Series S') {
    return seriesSImage
  }
  if (consoleType === 'Series X') {
    return seriesXImage
  }
  return seriesXImage
}

function resolvePowerState(
  rawState: HostSummary['powerState'],
): 'on' | 'standby' | 'off' | 'unknown' {
  if (rawState === 'On') {
    return 'on'
  }
  if (rawState === 'ConnectedStandby' || rawState === 'Connected') {
    return 'standby'
  }
  if (rawState === 'Off') {
    return 'off'
  }
  return 'unknown'
}

function resolveHostStatus(host: HostSummary): string {
  const powerState = resolvePowerState(host.powerState)
  if (powerState === 'on') {
    return t('homePage.consoleCard.states.availableOn')
  }
  if (powerState === 'standby') {
    return t('homePage.consoleCard.states.availableStandby')
  }
  if (powerState === 'off') {
    return t('homePage.consoleCard.states.unavailableOff')
  }
  return t('homePage.consoleCard.states.unavailableUnknown')
}

function resolveHostDescription(host: HostSummary): string {
  const powerState = resolvePowerState(host.powerState)
  if (powerState === 'on' && host.consoleStreamingEnabled !== false) {
    return t('homePage.consoleCard.actions.readyToPlay')
  }
  if (powerState === 'standby' && host.remoteManagementEnabled) {
    return t('homePage.consoleCard.actions.canWakeRemotely')
  }
  if (powerState === 'off') {
    return t('homePage.consoleCard.actions.checkNetwork')
  }
  return t('homePage.consoleCard.actions.notReady')
}

const hostCards = computed<HostCardViewModel[]>(() =>
  hosts.value.map((host, index) => ({
    host,
    sourceIndex: index,
    consoleType: resolveHostConsoleType(host),
    hostKey: resolveHostId(host, index),
    nodeId: `xhome.host.${resolveHostId(host, index)}`,
    title: resolveHostTitle(host),
    imageSrc: resolveConsoleImage(resolveHostConsoleType(host)),
    status: resolveHostStatus(host),
    description: resolveHostDescription(host),
  })),
)

const selectedCard = computed(() => hostCards.value.find(card => card.hostKey === selectedHostKey.value) ?? hostCards.value[0])
const canStreamSelected = computed(() => {
  const host = selectedCard.value?.host
  if (!host)
    return false
  const state = resolvePowerState(host.powerState)
  return (state === 'on' && host.consoleStreamingEnabled !== false)
    || (state === 'standby' && host.remoteManagementEnabled === true)
})
const primaryLabel = computed(() => !canStreamSelected.value
  ? t('homePage.hero.play')
  : t(resolvePowerState(selectedCard.value?.host.powerState) === 'standby' ? 'homePage.hero.wake' : 'homePage.hero.play'))
useShellPageHints(() => ({ status: loadError.value || selectedCard.value?.description, error: !!loadError.value }))

function handlePrimaryAction(): void {
  const card = selectedCard.value
  if (card && canStreamSelected.value)
    handleSelectHost(card.host, card.sourceIndex)
  else handleRefresh()
}

async function loadHosts(): Promise<void> {
  if (isLoading.value) {
    return
  }

  isLoading.value = true
  loadError.value = ''
  try {
    const result = await rpc.data.getHosts()
    hosts.value = Array.isArray(result) ? result : []
  }
  catch (error) {
    devWarn('[Home] load hosts failed:', error)
    loadError.value = t('homePage.hero.error')
  }
  finally {
    isLoading.value = false
  }
}

function handleSelectHost(host: HostSummary, index: number): void {
  void router.push({
    name: 'xhome-stream',
    params: {
      targetId: resolveStreamTargetId(host, index),
    },
    query: {
      name: resolveHostTitle(host),
      consoleId: resolveConsoleCommandId(host, index),
      powerState: host.powerState ?? '',
      remoteManagementEnabled: host.remoteManagementEnabled === true ? '1' : '0',
    },
  })
}

function handleRefresh(): void {
  void loadHosts()
}

onMounted(() => {
  void loadHosts()
})

usePageFocus(() => pageRef.value, () => selectedHostKey.value, () => SPATIAL_NAV_NODE_IDS.pagePrimary.xhome, false)
</script>

<template>
  <section ref="pageRef" class="home-page ui-page-shell" :aria-label="t('homePage.hero.title')">
    <header class="home-page__header">
      <div>
        <h1 class="home-page__title">
          {{ t('homePage.hero.title') }}
        </h1><p class="home-page__hint">
          {{ t('homePage.hero.hint') }}
        </p>
      </div>
      <Focusable id="xhome.refresh" as="button" type="button" class="home-page__refresh" :aria-label="t('homePage.refresh')" :title="t('homePage.refresh')" :disabled="isLoading" @click="handleRefresh">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5" /><path d="M6.2 6.2A8 8 0 0 1 20 12M4 12a8 8 0 0 0 13.8 5.8" /></svg>
      </Focusable>
    </header>
    <div v-if="isLoading && !selectedCard" class="home-page__loading">
      <BrandedLoading :label="t('homePage.loading')" />
    </div>
    <template v-else-if="selectedCard">
      <div class="home-page__hero-edge">
        <Transition name="panel-slide" mode="out-in">
          <section :key="selectedCard.hostKey" class="home-page__hero" :aria-label="selectedCard.title">
            <div class="home-page__hero-copy">
              <p class="home-page__eyebrow">
                REMOTE PLAY · {{ selectedCard.consoleType }}
              </p>
              <h2 class="home-page__console-name">
                {{ selectedCard.title }}
              </h2>
              <p class="home-page__status" :data-power="resolvePowerState(selectedCard.host.powerState)">
                {{ selectedCard.status }}
              </p>
              <p class="home-page__description">
                {{ selectedCard.description }}
              </p>
              <div class="home-page__actions">
                <Focusable :id="SPATIAL_NAV_NODE_IDS.pagePrimary.xhome" as="button" type="button" class="home-page__primary ui-action-button ui-action-button--brand" :disabled="isLoading || !canStreamSelected" @click="handlePrimaryAction">
                  {{ primaryLabel }}
                </Focusable>
                <Focusable v-if="selectedCard.host.id || selectedCard.host.deviceId" id="xhome.installed-games" as="button" type="button" class="home-page__library ui-action-button" @click="router.push({ name: 'console-games', params: { consoleKey: selectedCard.host.id || selectedCard.host.deviceId }, query: { from: '/xhome' } })">
                  已安装游戏
                </Focusable>
              </div>
            </div>
            <img class="home-page__console-art" :src="selectedCard.imageSrc" :alt="selectedCard.consoleType">
          </section>
        </Transition>
      </div>
      <section v-if="hostCards.length > 1" class="home-page__devices" :aria-label="t('homePage.hero.switch')">
        <header class="home-page__devices-header">
          <h2>{{ t('homePage.hero.switch') }}</h2><span>{{ t('homePage.railHint', { count: hosts.length }) }}</span>
        </header>
        <div class="home-page__device-grid">
          <ConsoleStatusCard v-for="card in hostCards" :id="card.nodeId" :key="card.hostKey" :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell" :title="card.title" :status="card.status" :description="card.description" :image-src="card.imageSrc" :image-alt="card.consoleType" :selected="card.hostKey === selectedCard.hostKey" :aria-label="t('homePage.consoleCard.ariaLabel', { name: card.title })" @click="selectedHostKey = card.hostKey" />
        </div>
      </section>
    </template>
    <section v-else class="home-page__empty-state" :aria-label="t('homePage.empty')">
      <p class="home-page__empty-copy" :role="loadError ? 'alert' : undefined">
        {{ loadError || t('homePage.empty') }}
      </p>
      <LibraryRefresh :id="SPATIAL_NAV_NODE_IDS.pagePrimary.xhome" :busy="isLoading" @refresh="handleRefresh" />
    </section>
  </section>
</template>

<style scoped>
.home-page { display: flex; flex-direction: column; gap: var(--space-24); min-height: 100%; }
.home-page__header, .home-page__devices-header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-16); }
.home-page__title { font-size: 24px; line-height: 32px; font-weight: 500; letter-spacing: -.02em; }
.home-page__hint { margin-top: var(--space-8); color: var(--color-text-secondary); font-size: 12px; line-height: 18px; }
.home-page__refresh { display: grid; place-items: center; flex: 0 0 36px; width: 36px; height: 36px; padding: 8px; border: 1px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: var(--color-text-primary); cursor: pointer; }
.home-page__refresh svg { width: 20px; height: 20px; }
.home-page__refresh:disabled { opacity: .5; cursor: default; }
.home-page__hero-edge { min-height: var(--ui-console-hero-height); border-radius: var(--ui-radius-panel); }
.home-page__hero { display: grid; grid-template-columns: 1.15fr 1fr; align-items: center; gap: var(--space-24); min-height: var(--ui-console-hero-height); padding: var(--ui-console-hero-padding); border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-panel); background: var(--ui-console-hero-bg); backdrop-filter: blur(16px); box-shadow: inset 0 1px 0 var(--ui-material-edge), var(--ui-shadow-low); }
.home-page__hero-copy { min-width: 0; }
.home-page__eyebrow { margin-bottom: var(--space-16); color: var(--color-text-secondary); font-size: 12px; line-height: 16px; letter-spacing: .08em; }
.home-page__console-name { font-size: var(--ui-console-hero-title-size); line-height: 1.18; font-weight: 500; letter-spacing: -.02em; overflow-wrap: anywhere; }
.home-page__status { display: flex; align-items: center; gap: var(--space-8); margin-top: var(--space-12); color: var(--color-text-secondary); font-size: 14px; line-height: 20px; }
.home-page__status::before { content: ''; width: 6px; height: 6px; flex-shrink: 0; border-radius: 50%; background: var(--color-text-tertiary); }
.home-page__status[data-power='on']::before, .home-page__status[data-power='standby']::before { background: var(--ui-status-positive); }
.home-page__description { margin-top: var(--space-4); color: var(--color-text-secondary); font-size: 12px; line-height: 18px; }
.home-page__actions { display: flex; align-items: center; gap: var(--space-12); flex-wrap: wrap; margin-top: var(--space-24); }
.home-page__library { min-width: 0; border: 0; background: color-mix(in srgb, var(--color-text-primary) 14%, var(--color-surface-1)); }
.home-page__library:hover { background: color-mix(in srgb, var(--color-text-primary) 20%, var(--color-surface-1)); }
.home-page__library[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.home-page__primary { min-width: 0; }
.home-page__primary[data-focused='true'], .home-page__refresh[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.home-page__primary:hover { background: var(--btn-primary-bg-hover); }
.home-page__refresh:hover { background: var(--ui-material-hover); }
.home-page__console-art { width: 100%; height: var(--ui-console-hero-image-height); object-fit: contain; filter: drop-shadow(12px 16px 12px color-mix(in srgb, var(--neutral-0) 40%, transparent)); }
.home-page__devices { display: flex; flex-direction: column; gap: var(--space-16); }
.home-page__devices-header h2 { font-size: 18px; line-height: 24px; font-weight: 500; }
.home-page__devices-header span { color: var(--color-text-secondary); font-size: 12px; line-height: 18px; }
.home-page__device-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-16); }
.home-page__loading, .home-page__empty-state { flex: 1; min-height: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--space-24); }
.home-page__empty-copy { max-width: 440px; color: var(--color-text-secondary); text-align: center; font-size: 14px; line-height: 1.5; }
@media(max-height:680px) { .home-page { gap: var(--space-16); } .home-page__actions { margin-top: var(--space-16); } .home-page__eyebrow { margin-bottom: var(--space-12); } }
@media(max-width:700px) { .home-page__header { flex-wrap: wrap; } .home-page__title { font-size: 24px; line-height: 32px; } .home-page__hero { grid-template-columns: 1fr .55fr; gap: var(--space-12); padding: var(--space-16); } .home-page__console-name { font-size: 24px; } .home-page__console-art { height: 160px; } .home-page__device-grid { grid-template-columns: minmax(0, 1fr); } }
@media(max-width:370px) { .home-page__hero { grid-template-columns: minmax(0, 1fr) .35fr; gap: var(--space-8); padding: var(--space-12); } .home-page__console-name { font-size: 22px; } .home-page__console-art { height: 144px; } }
</style>
