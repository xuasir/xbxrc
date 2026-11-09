import { computed, onActivated, onDeactivated, onUnmounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { rpc } from '@/services/rpc'
import { libraryEpoch } from './use-library-query'

type Host = Awaited<ReturnType<typeof rpc.data.getHosts>>[number]
export function useConsole(key: () => string) {
  const hosts = ref<Host[]>([])
  const loading = ref(false)
  const router = useRouter()
  let active = true
  let generation = 0
  let pending: { key: string, epoch: number, promise: Promise<void> } | null = null
  function refresh(): Promise<void> {
    const identity = key()
    const epoch = libraryEpoch.value
    if (!identity || !active)
      return Promise.resolve()
    if (pending?.key === identity && pending.epoch === epoch)
      return pending.promise
    const request = ++generation
    loading.value = true
    const promise = (async () => {
      try {
        const result = await rpc.data.getHosts()
        if (request === generation && epoch === libraryEpoch.value && identity === key())
          hosts.value = result
      }
      catch {
        // 当前设备查询失败时，串流入口等待下一次有效状态。
        if (request === generation && epoch === libraryEpoch.value && identity === key())
          hosts.value = []
      }
      finally {
        if (request === generation) {
          loading.value = false
          pending = null
        }
      }
    })()
    pending = { key: identity, epoch, promise }
    return promise
  }
  watch(() => [key(), libraryEpoch.value], () => {
    generation++
    pending = null
    loading.value = false
    hosts.value = []
    void refresh()
  }, { immediate: true })
  onActivated(() => {
    active = true
    void refresh()
  })
  function deactivate() {
    active = false
    generation++
    pending = null
    loading.value = false
  }
  onDeactivated(deactivate)
  onUnmounted(deactivate)
  const host = computed(() => hosts.value.find(h => h.id === key() || h.deviceId === key()))
  // 刷新期间沿用已知设备能力，UI 通过 busy 合并确认并保持导航候选。
  const streamAvailable = computed(() => !!host.value && !!(host.value.serverId || host.value.id) && ((host.value.powerState === 'On' && host.value.consoleStreamingEnabled !== false) || (['Connected', 'ConnectedStandby'].includes(host.value.powerState ?? '') && host.value.remoteManagementEnabled === true)))
  const canStream = computed(() => !loading.value && streamAvailable.value)
  function stream() {
    const h = host.value
    if (!h || !canStream.value)
      return
    void router.push({ name: 'xhome-stream', params: { targetId: h.serverId || h.id }, query: { name: h.name || h.deviceName || 'Xbox', consoleId: h.id || h.deviceId, powerState: h.powerState, remoteManagementEnabled: h.remoteManagementEnabled ? '1' : '0' } })
  }
  return { host, streamAvailable, canStream, stream, refresh, loading }
}
