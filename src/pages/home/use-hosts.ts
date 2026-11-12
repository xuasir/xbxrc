import { onActivated, onBeforeUnmount, onDeactivated, onMounted } from 'vue'
import { events } from '@/services/events'
import { rpc } from '@/services/rpc'
import { devWarn } from '@/shared/dev-log'
import { HostsStore } from './hosts-store'

export function useHosts() {
  const store = new HostsStore({
    snapshot: () => rpc.data.getHostsSnapshot(),
    refresh: () => rpc.data.refreshHosts(),
  })
  let active = true
  let enabled = true
  let disposed = false
  let loggedError: unknown = null
  let timer: ReturnType<typeof setTimeout> | undefined
  const disposers: (() => void)[] = []

  function stopTimer(): void {
    clearTimeout(timer)
    timer = undefined
  }

  function schedule(): void {
    stopTimer()
    if (active && enabled && !disposed)
      timer = setTimeout(() => void update(), Math.min(60_000, Math.max(0, store.nextCheckAt - Date.now())))
  }

  async function update(force = false): Promise<void> {
    if (!active || !enabled || disposed)
      return
    stopTimer()
    await store.fetch(force)
    if (store.error.value && store.error.value !== loggedError)
      devWarn('[Home] load hosts failed:', store.error.value)
    loggedError = store.error.value
    schedule()
  }

  onMounted(() => {
    disposers.push(events.on('data.hostsChanged', (event) => {
      store.invalidate(event.scopeKey)
      void update()
    }), events.on('auth.sessionReady', () => {
      enabled = true
      store.clear()
      void update()
    }), events.on('auth.stateChanged', (state) => {
      enabled = state.isAuthenticated
      if (!enabled) {
        stopTimer()
        store.clear()
      }
      else {
        void update()
      }
    }))
    void update()
  })
  onActivated(() => {
    active = true
    void update()
  })
  onDeactivated(() => {
    active = false
    stopTimer()
  })
  onBeforeUnmount(() => {
    active = false
    disposed = true
    stopTimer()
    store.clear()
    for (const dispose of disposers)
      dispose()
  })

  return { store, refresh: () => update(true) }
}
