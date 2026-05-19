import type { UpdaterProgressEvent } from '@shared/events/updater'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { events } from '../services/events'
import { rpc } from '../services/rpc'

export type AppUpdaterState
  = | 'idle'
    | 'checking'
    | 'available'
    | 'downloading'
    | 'installing'
    | 'installed'
    | 'error'

export type UpdateChannel = 'stable' | 'beta'

export function useAppUpdater() {
  const state = ref<AppUpdaterState>('idle')
  const channel = ref<UpdateChannel>('stable')
  const currentVersion = ref('')
  const targetVersion = ref<string | null>(null)
  const releaseNotes = ref<string | null>(null)
  const errorMessage = ref<string | null>(null)
  const downloadProgress = ref(0)
  const downloadTotal = ref<number | null>(null)

  const statusText = computed(() => {
    switch (state.value) {
      case 'idle':
        return ''
      case 'checking':
        return 'checking'
      case 'available':
        return 'available'
      case 'downloading':
        return 'downloading'
      case 'installing':
        return 'installing'
      case 'installed':
        return 'installed'
      case 'error':
        return 'error'
      default:
        return ''
    }
  })

  const progressPercent = computed(() => {
    if (downloadTotal.value == null || downloadTotal.value <= 0) {
      return null
    }
    return Math.min(100, Math.round((downloadProgress.value / downloadTotal.value) * 100))
  })

  async function refreshCurrentVersion() {
    currentVersion.value = await rpc.app.getVersion()
  }

  async function loadChannel() {
    const result = await rpc.updater.getChannel()
    channel.value = result.channel
  }

  async function setChannel(next: UpdateChannel) {
    await rpc.updater.setChannel({ channel: next })
    channel.value = next
    state.value = 'idle'
    targetVersion.value = null
    releaseNotes.value = null
    errorMessage.value = null
  }

  async function checkForUpdate() {
    state.value = 'checking'
    errorMessage.value = null
    targetVersion.value = null
    releaseNotes.value = null
    downloadProgress.value = 0
    downloadTotal.value = null

    try {
      const result = await rpc.updater.check()
      currentVersion.value = result.currentVersion
      if (!result.updateAvailable || !result.version) {
        state.value = 'idle'
        return
      }
      targetVersion.value = result.version
      releaseNotes.value = result.notes
      state.value = 'available'
    }
    catch (error) {
      errorMessage.value = error instanceof Error ? error.message : String(error)
      state.value = 'error'
    }
  }

  function handleProgress(event: UpdaterProgressEvent) {
    if (event.event === 'started') {
      state.value = 'downloading'
      downloadProgress.value = 0
      downloadTotal.value = event.contentLength
    }
    else if (event.event === 'progress') {
      state.value = 'downloading'
      downloadProgress.value = event.downloaded
      downloadTotal.value = event.contentLength ?? downloadTotal.value
    }
    else if (event.event === 'finished') {
      state.value = 'installing'
    }
  }

  async function downloadAndInstall() {
    if (state.value !== 'available') {
      return
    }
    state.value = 'downloading'
    errorMessage.value = null
    try {
      await rpc.updater.downloadAndInstall()
      state.value = 'installed'
    }
    catch (error) {
      errorMessage.value = error instanceof Error ? error.message : String(error)
      state.value = 'error'
    }
  }

  async function relaunch() {
    await rpc.updater.relaunch()
  }

  let unsubscribeProgress: (() => void) | null = null

  onMounted(async () => {
    unsubscribeProgress = events['updater.progress'](handleProgress)
    await Promise.all([refreshCurrentVersion(), loadChannel()])
  })

  onUnmounted(() => {
    unsubscribeProgress?.()
    unsubscribeProgress = null
  })

  return {
    state,
    channel,
    currentVersion,
    targetVersion,
    releaseNotes,
    errorMessage,
    statusText,
    progressPercent,
    downloadProgress,
    downloadTotal,
    setChannel,
    checkForUpdate,
    downloadAndInstall,
    relaunch,
    refreshCurrentVersion,
  }
}
