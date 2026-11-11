import { onBeforeUnmount, onDeactivated, ref } from 'vue'
import { rpc } from '@/services/rpc'

export function useCloudWait() {
  const product = ref('')
  const seconds = ref<number | null>(null)
  const loading = ref(false)
  const error = ref('')
  let timer: ReturnType<typeof setTimeout> | undefined
  let request = ''
  let generation = 0
  function cancel() {
    generation++
    clearTimeout(timer)
    if (request)
      void rpc.data.cancelCloudWait({ requestId: request }).catch(() => {})
    request = ''
    product.value = ''
    loading.value = false
  }
  function focus(id: string, allowed = true) {
    if (product.value === id)
      return
    cancel()
    if (!allowed)
      return
    product.value = id
    seconds.value = null
    error.value = ''
    const current = generation
    timer = setTimeout(async () => {
      request = crypto.randomUUID()
      loading.value = true
      try {
        const value = await rpc.data.getCloudWait({ productId: id, requestId: request })
        if (generation === current)
          seconds.value = value.seconds
      }
      catch (cause) {
        if (generation === current)
          error.value = cause instanceof Error ? cause.message : String(cause)
      }
      finally {
        if (generation === current) {
          request = ''
          loading.value = false
        }
      }
    }, 500)
  }
  onDeactivated(cancel)
  onBeforeUnmount(cancel)
  return { product, seconds, loading, error, focus, cancel }
}
