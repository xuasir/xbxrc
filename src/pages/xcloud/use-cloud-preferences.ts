import type { CloudPreferences, CloudTitle } from '@/shared/rpc/cloud'
import { ref } from 'vue'
import { events } from '@/services/events'
import { rpc } from '@/services/rpc'

const empty = (): CloudPreferences => ({ favorites: [], pinned: [], section: '', category: '', sort: 'name' })
const preferences = ref<CloudPreferences>(empty())
const scopeKey = ref('')
const error = ref('')
let generation = 0
let pending: Promise<void> | undefined
let queue = Promise.resolve()
let initialized = false
export function useCloudPreferences() {
  if (!initialized) {
    initialized = true
    events.on('auth.stateChanged', (state) => {
      if (!state.isAuthenticated) {
        generation++
        pending = undefined
        scopeKey.value = ''
        preferences.value = empty()
      }
    })
    events.on('auth.sessionReady', () => {
      generation++
      pending = undefined
      scopeKey.value = ''
      preferences.value = empty()
    })
  }
  function load() {
    if (pending)
      return pending
    const request = generation
    const operation = queue.then(async () => {
      if (request !== generation)
        return
      const [key, value] = await rpc.data.getCloudPreferences()
      if (request !== generation)
        return
      if (scopeKey.value && scopeKey.value !== key)
        generation++
      scopeKey.value = key
      preferences.value = value
      error.value = ''
    }).catch((cause) => {
      if (request === generation)
        error.value = cause instanceof Error ? cause.message : String(cause)
    }).finally(() => {
      if (pending === operation)
        pending = undefined
    })
    pending = operation
    return operation
  }
  async function update(patch: Partial<CloudPreferences>) {
    if (!scopeKey.value)
      await load()
    if (!scopeKey.value)
      return
    const key = scopeKey.value
    const request = generation
    preferences.value = { ...preferences.value, ...patch }
    const value = JSON.parse(JSON.stringify(preferences.value)) as CloudPreferences
    queue = queue.then(async () => {
      if (request !== generation || key !== scopeKey.value)
        return
      try {
        await rpc.data.setCloudPreferences({ scopeKey: key, preferences: value })
        error.value = ''
      }
      catch (cause) {
        if (request === generation)
          error.value = cause instanceof Error ? cause.message : String(cause)
      }
    })
    return queue
  }
  async function favorite(title: CloudTitle) {
    if (!scopeKey.value)
      await load()
    const exists = preferences.value.favorites.some(item => item.productId === title.productId)
    return update({ favorites: exists ? preferences.value.favorites.filter(item => item.productId !== title.productId) : [...preferences.value.favorites, title], pinned: exists ? preferences.value.pinned.filter(id => id !== title.productId) : preferences.value.pinned })
  }
  async function pin(id: string) {
    if (!scopeKey.value)
      await load()
    return update({ pinned: preferences.value.pinned.includes(id) ? preferences.value.pinned.filter(item => item !== id) : [...preferences.value.pinned, id] })
  }
  return { preferences, scopeKey, error, load, update, favorite, pin }
}
