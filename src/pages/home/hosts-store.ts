import type { HostsSnapshot } from '@/shared/rpc/hosts'
import { ref, shallowRef } from 'vue'

interface HostsLoader {
  snapshot: () => Promise<HostsSnapshot>
  refresh: () => Promise<HostsSnapshot>
}

/** 保留 Rust 快照的页面投影，时效与账户身份随 RPC 结果同步。 */
export class HostsStore {
  readonly snapshot = shallowRef<HostsSnapshot | null>(null)
  readonly loading = ref(false)
  readonly hasLoaded = ref(false)
  readonly error = shallowRef<unknown>(null)
  private epoch = 0
  private invalidation = 0
  private recheckSnapshot = false
  private retryAt = 0
  private pending: Promise<boolean> | null = null

  constructor(private readonly loader: HostsLoader) {}

  get nextCheckAt(): number {
    return Math.max(this.retryAt, this.snapshot.value?.freshUntil ?? 0)
  }

  clear(): void {
    this.epoch++
    this.invalidation++
    this.pending = null
    this.snapshot.value = null
    this.loading.value = false
    this.hasLoaded.value = false
    this.error.value = null
    this.retryAt = 0
    this.recheckSnapshot = false
  }

  invalidate(scope: string): void {
    if (this.snapshot.value && this.snapshot.value.scopeKey !== scope)
      return
    this.invalidation++
    this.retryAt = 0
    this.recheckSnapshot = true
    if (this.snapshot.value)
      this.snapshot.value = { ...this.snapshot.value, freshUntil: 0 }
  }

  fetch(force = false): Promise<boolean> {
    if (this.pending)
      return this.pending
    if (!force && this.nextCheckAt > Date.now())
      return Promise.resolve(false)
    const epoch = this.epoch
    const invalidation = this.invalidation
    const refresh = force || (!!this.snapshot.value && !this.recheckSnapshot)
    this.loading.value = true
    // 在途 Promise 先登记，同步异常也会走相同的清理与重试路径。
    const pending = Promise.resolve().then(() => refresh ? this.loader.refresh() : this.loader.snapshot()).then((result) => {
      if (epoch !== this.epoch || invalidation !== this.invalidation)
        return false
      this.snapshot.value = result
      this.error.value = null
      this.retryAt = 0
      this.recheckSnapshot = false
      return true
    }).catch((error: unknown) => {
      if (epoch === this.epoch && invalidation === this.invalidation) {
        this.error.value = error
        this.retryAt = Date.now() + 15_000
      }
      return false
    }).finally(() => {
      if (epoch === this.epoch && this.pending === pending) {
        if (invalidation === this.invalidation)
          this.hasLoaded.value = true
        this.loading.value = false
        this.pending = null
      }
    })
    this.pending = pending
    return pending
  }
}
