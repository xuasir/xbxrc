import type { UpdaterProgressEvent } from '@shared/events/updater'
import { computed, ref } from 'vue'
import { events } from '../services/events'
import { rpc } from '../services/rpc'

export type AppUpdaterState
  = | 'idle'
    | 'current'
    | 'checking'
    | 'available'
    | 'downloading'
    | 'installing'
    | 'installed'
    | 'error'

export type UpdateChannel = 'stable' | 'beta'

function createAppUpdater() {
  const state = ref<AppUpdaterState>('idle')
  const channel = ref<UpdateChannel>('stable')
  const currentVersion = ref('')
  const targetVersion = ref<string | null>(null)
  const releaseNotes = ref<string | null>(null)
  const releaseDate = ref<string | null>(null)
  const checkedAt = ref<number | null>(null)
  const channelError = ref<string | null>(null)
  const channelLoading = ref(false)
  const channelReady = ref(false)
  const errorMessage = ref<string | null>(null)
  const downloadProgress = ref(0)
  const downloadTotal = ref<number | null>(null)

  const statusText = computed(() => {
    switch (state.value) {
      case 'idle':
        return ''
      case 'current':
        return 'current'
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

  const busy = computed(() => ['checking', 'downloading', 'installing'].includes(state.value))
  const channelSaving = ref(false)
  let channelRevision = 0

  async function refreshCurrentVersion() {
    try {
      currentVersion.value = await rpc.app.getVersion()
    }
    catch { currentVersion.value = '—' }
  }

  async function loadChannel() {
    if (channelLoading.value || channelSaving.value || busy.value)
      return
    channelLoading.value = true
    const revision = channelRevision
    try {
      const result = await rpc.updater.getChannel()
      if (revision !== channelRevision || channelSaving.value)
        return
      channel.value = result.channel
      channelReady.value = true
      channelError.value = null
    }
    catch (error) {
      if (revision === channelRevision)
        channelError.value = error instanceof Error ? error.message : String(error)
    }
    finally { channelLoading.value = false }
  }

  async function setChannel(next: UpdateChannel) {
    if (busy.value || channelSaving.value || state.value === 'installed')
      return
    channelSaving.value = true
    ++channelRevision
    try {
      await rpc.updater.setChannel({ channel: next })
    }
    catch (error) {
      channelError.value = error instanceof Error ? error.message : String(error)
      throw error
    }
    finally { channelSaving.value = false }
    channel.value = next
    channelReady.value = true
    channelError.value = null
    checkedAt.value = null
    state.value = 'idle'
    targetVersion.value = null
    releaseNotes.value = null
    releaseDate.value = null
    errorMessage.value = null
  }

  async function checkForUpdate() {
    if (busy.value || channelSaving.value || state.value === 'installed')
      return
    state.value = 'checking'
    errorMessage.value = null
    targetVersion.value = null
    releaseNotes.value = null
    releaseDate.value = null
    downloadProgress.value = 0
    downloadTotal.value = null

    try {
      const result = await rpc.updater.check()
      currentVersion.value = result.currentVersion
      checkedAt.value = Date.now()
      releaseDate.value = result.date ?? null
      if (!result.updateAvailable || !result.version) {
        state.value = 'current'
        return
      }
      targetVersion.value = result.version
      releaseNotes.value = result.notes?.trim() || null
      state.value = 'available'
    }
    catch (error) {
      errorMessage.value = error instanceof Error ? error.message : String(error)
      state.value = 'error'
    }
  }

  function handleProgress(event: UpdaterProgressEvent) {
    if (state.value !== 'downloading' && state.value !== 'installing')
      return
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
    if (state.value !== 'available' && !(state.value === 'error' && targetVersion.value)) {
      return
    }
    state.value = 'downloading'
    errorMessage.value = null
    downloadProgress.value = 0
    downloadTotal.value = null
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
    errorMessage.value = null
    try {
      await rpc.updater.relaunch()
    }
    catch (error) { errorMessage.value = error instanceof Error ? error.message : String(error) }
  }

  // 更新任务拥有应用级状态，分类组件仅展示，进度订阅跨页面持续存在。
  const unsubscribeProgress = events.on('updater.progress', handleProgress)
  void Promise.all([refreshCurrentVersion(), loadChannel()])
  if (import.meta.hot)
    import.meta.hot.dispose(() => unsubscribeProgress())

  return {
    state,
    busy,
    channelError,
    channelLoading,
    channelReady,
    channelSaving,
    releaseDate,
    checkedAt,
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
    loadChannel,
  }
}

let sharedUpdater: ReturnType<typeof createAppUpdater> | undefined
export function useAppUpdater() {
  sharedUpdater ??= createAppUpdater()
  return sharedUpdater
}
