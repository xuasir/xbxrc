import type { Playtime, QueryResult } from '@/shared/rpc/library'
import { onActivated, onDeactivated, onUnmounted, ref, watch } from 'vue'
import { diagnosticIssueId, retainDiagnosticIssue } from '@/services/diagnostics'
import { rpc } from '@/services/rpc'
import { formatMinutes } from './library-view'
import { libraryAccountId, libraryEpoch } from './use-library-query'

type PlaytimeLoader = (params: { targetXuid?: string, xboxTitleIds: string[] }) => Promise<QueryResult<Playtime[]>>

export function usePlaytimes(ids: () => string[], target: () => string, enabled: () => boolean, loader: PlaytimeLoader = params => rpc.data.getPlaytimes(params)) {
  const values = ref(new Map<string, number | null>())
  const failures = ref(new Map<string, string>())
  const loading = ref(false)
  const issueId = ref<string>()
  const error = ref('')
  let active = true
  let generation = 0
  let pending: Promise<void> | null = null
  let retainingIssue = false

  function cancel() {
    generation++
    pending = null
    loading.value = false
  }
  function reset() {
    cancel()
    values.value.clear()
    failures.value.clear()
    error.value = ''
    issueId.value = undefined
  }
  function load(): Promise<void> {
    if (!active || !enabled() || !ids().length)
      return Promise.resolve()
    if (pending)
      return pending
    if (ids().every(id => values.value.has(id)))
      return Promise.resolve()
    const request = ++generation
    const epoch = libraryEpoch.value
    const identity = target()
    const current = () => request === generation && epoch === libraryEpoch.value && identity === target() && active && enabled()
    const attempted = new Set<string>()
    loading.value = true
    error.value = ''
    pending = Promise.resolve().then(async () => {
      try {
        // 重复激活共用当前请求，翻页只补查新增 ID，缺失值保留为明确的 null。
        while (current()) {
          const batch = [...new Set(ids())].filter(id => !values.value.has(id) && !attempted.has(id)).slice(0, 100)
          if (!batch.length)
            break
          for (const id of batch) attempted.add(id)
          try {
            const result = await loader({ targetXuid: identity || undefined, xboxTitleIds: batch })
            if (!current() || (libraryAccountId.value && result.accountId !== libraryAccountId.value) || result.targetId !== (identity || result.accountId))
              return
            const returned = new Map(result.data.map(item => [item.xboxTitleId, item]))
            for (const id of batch) {
              const item = returned.get(id)
              if (item?.error) {
                failures.value.set(id, item.error)
                error.value = item.error
                if (!issueId.value && !retainingIssue) {
                  retainingIssue = true
                  void retainDiagnosticIssue(undefined, 'library').then((id) => {
                    if (current())
                      issueId.value = id
                  }).finally(() => { retainingIssue = false })
                }
              }
              else {
                values.value.set(id, item?.minutes ?? null)
                failures.value.delete(id)
              }
            }
          }
          catch (cause) {
            if (!current())
              return
            const message = cause instanceof Error ? cause.message : '时长查询暂不可用'
            issueId.value = diagnosticIssueId(cause)
            error.value = message
            for (const id of batch) failures.value.set(id, message)
          }
        }
      }
      finally {
        if (request === generation) {
          pending = null
          loading.value = false
        }
      }
    })
    return pending
  }
  function label(id: string): string {
    if (values.value.has(id))
      return formatMinutes(values.value.get(id))
    if (failures.value.has(id))
      return '查询失败'
    return loading.value ? '查询中…' : '待查询'
  }
  watch(() => [target(), libraryEpoch.value], reset, { flush: 'sync' })
  watch(() => [ids(), enabled(), target(), libraryEpoch.value], () => {
    if (active && enabled())
      void load()
    else cancel()
  }, { immediate: true })
  onActivated(() => {
    active = true
    void load()
  })
  const deactivate = () => {
    active = false
    cancel()
  }
  onDeactivated(deactivate)
  onUnmounted(deactivate)
  return { issueId, values, loading, error, failures, load, reset, label }
}
