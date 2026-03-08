<script setup lang="ts">
import { Focusable } from '@spatial-navigation/vue'
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import seriesXImage from '../assets/console/series-x.png'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import ConsoleStatusCard from '../components/common/ConsoleStatusCard.vue'
import HorizontalListRail from '../components/common/HorizontalListRail.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'

const { t } = useI18n()
const router = useRouter()

type HostSummary = Awaited<ReturnType<typeof rpc.data.getHosts>>[number]

interface HostCardViewModel {
  host: HostSummary
  hostKey: string
  nodeId: string
  title: string
  imageSrc: string
  status: string
  description: string
}

const isLoading = ref(false)
const hosts = ref<HostSummary[]>([])

function resolveHostId(host: HostSummary, index: number): string {
  return host.serverId ?? host.id ?? host.deviceId ?? `host-${index}`
}

function resolveStreamTargetId(host: HostSummary, index: number): string {
  return host.serverId ?? host.id ?? host.deviceId ?? `host-${index}`
}

function resolveHostTitle(host: HostSummary): string {
  const title = host.name ?? host.deviceName
  return typeof title === 'string' && title.trim() !== '' ? title.trim() : 'Xbox'
}

function resolveHostConsoleType(host: HostSummary): string {
  return typeof host.consoleType === 'string' && host.consoleType !== '' ? host.consoleType : 'Xbox'
}

function resolveConsoleImage(consoleType: string): string {
  void consoleType
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
    hostKey: resolveHostId(host, index),
    nodeId:
      index === 0
        ? SPATIAL_NAV_NODE_IDS.pagePrimary.xhome
        : `xhome.host.${resolveHostId(host, index)}`,
    title: resolveHostTitle(host),
    imageSrc: resolveConsoleImage(resolveHostConsoleType(host)),
    status: resolveHostStatus(host),
    description: resolveHostDescription(host),
  })),
)

async function loadHosts(): Promise<void> {
  if (isLoading.value) {
    return
  }

  isLoading.value = true
  try {
    const result = await rpc.data.getHosts()
    hosts.value = Array.isArray(result) ? result : []
  }
  catch (error) {
    console.warn('[Home] load hosts failed:', error)
    hosts.value = []
  }
  finally {
    isLoading.value = false
  }
}

function buildHostNeighbors(index: number): Record<'up' | 'left' | 'right', string | undefined> {
  return {
    up: SPATIAL_NAV_NODE_IDS.topNav.xhome,
    left: index > 0 ? hostCards.value[index - 1]?.nodeId : undefined,
    right: index < hostCards.value.length - 1 ? hostCards.value[index + 1]?.nodeId : undefined,
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
</script>

<template>
  <section class="home-page ui-page-shell" aria-label="XHome Page">
    <div v-if="isLoading" class="home-page__loading">
      <BrandedLoading :label="t('homePage.loading')" />
    </div>

    <HorizontalListRail
      v-else-if="hostCards.length > 0"
      class="home-page__content"
      :title="t('homePage.railTitle')"
      :hint="t('homePage.railHint', { count: hosts.length })"
      :aria-label="t('homePage.railAriaLabel')"
    >
      <ConsoleStatusCard
        v-for="(card, index) in hostCards"
        :id="card.nodeId"
        :key="card.hostKey"
        :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
        :title="card.title"
        :status="card.status"
        :description="card.description"
        :image-src="card.imageSrc"
        :image-alt="card.title"
        :neighbors="buildHostNeighbors(index)"
        :aria-label="t('homePage.consoleCard.ariaLabel', { name: card.title })"
        :index="{ row: 0, col: index, order: index }"
        :on-click="() => handleSelectHost(card.host, index)"
      />
    </HorizontalListRail>

    <section v-else class="home-page__empty-state" :aria-label="t('homePage.empty')">
      <p class="home-page__empty-copy">
        {{ t('homePage.empty') }}
      </p>

      <Focusable
        :id="SPATIAL_NAV_NODE_IDS.pagePrimary.xhome"
        as="button"
        type="button"
        class="home-page__refresh-button"
        :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
        :neighbors="{ up: SPATIAL_NAV_NODE_IDS.topNav.xhome }"
        :aria-label="t('homePage.refresh')"
        :on-confirm="handleRefresh"
        @click="handleRefresh"
      >
        {{ t('homePage.refresh') }}
      </Focusable>
    </section>
  </section>
</template>

<style scoped>
.home-page {
  display: flex;
  flex-direction: column;
  gap: var(--ui-page-stack-gap);
  min-height: 100%;
}

.home-page__loading {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 0;
}

.home-page__content {
  flex: 1 1 auto;
  min-height: 0;
  padding: 10px 0 calc(var(--ui-space-4xl) + 8px) 0;
}

.home-page__empty-state {
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  gap: 18px;
  flex: 1 1 auto;
  width: min(100%, var(--ui-home-empty-width));
  margin: auto;
  min-height: 0;
  padding: 0;
  background: transparent;
  border: 0;
  box-shadow: none;
}

.home-page__empty-copy {
  max-width: var(--ui-home-empty-copy-max-width);
  font-size: var(--ui-home-empty-copy-size);
  line-height: 1.55;
  color: var(--color-text-secondary);
  text-align: center;
}

.home-page__refresh-button {
  min-width: var(--ui-home-action-min-width);
  min-height: var(--ui-home-action-min-height);
  padding: 0 18px;
  border: 1px solid var(--btn-border);
  border-radius: var(--btn-radius);
  background: var(--btn-primary-bg);
  color: var(--btn-primary-text);
  font-size: 14px;
  font-weight: var(--ui-font-weight-semibold);
  line-height: 1;
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.home-page__refresh-button:hover {
  transform: translateY(-1px);
  background: var(--btn-primary-bg-hover);
  box-shadow: 0 12px 24px rgba(22, 132, 57, 0.22);
}

.home-page__refresh-button[data-focused='true'] {
  border-color: var(--color-focus-ring);
  background: var(--btn-primary-bg-hover);
  color: var(--btn-primary-text);
  box-shadow: 0 0 0 var(--focus-ring-width) var(--color-focus-ring-outer) inset;
}

:global(html[data-ui-density='compact']) .home-page__content,
:global(html[data-ui-density='narrow']) .home-page__content {
  padding-top: 8px;
  padding-bottom: calc(var(--ui-space-4xl) + 4px);
}

:global(html[data-ui-density='compact']) .home-page__refresh-button,
:global(html[data-ui-density='narrow']) .home-page__refresh-button {
  font-size: 13px;
}
</style>
