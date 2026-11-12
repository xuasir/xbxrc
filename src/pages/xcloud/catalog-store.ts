import type { DataXcloudCatalogUpdatedRendererEvent } from '@/shared/events/data'
import type { CloudCatalog, CloudCatalogStatus, CloudTitle, CloudTitleSummary } from '@/shared/rpc/cloud'

interface CatalogLoader {
  snapshot: () => Promise<CloudCatalog>
  status: () => Promise<CloudCatalogStatus>
  refresh: () => Promise<CloudCatalog>
  summary: (productId: string) => Promise<CloudTitleSummary>
}
type Listener = (catalog: CloudCatalog | undefined) => void
function sameStatus(a: CloudCatalogStatus, b: CloudCatalogStatus): boolean {
  const keys = ['scopeKey', 'contentRevision', 'freshUntil', 'cacheState', 'updatedAt', 'refreshing', 'access', 'error'] as const
  const sources = a.sources ?? {}
  const other = b.sources ?? {}
  return keys.every(key => a[key] === b[key]) && Object.keys(sources).length === Object.keys(other).length && Object.keys(sources).every(key => sources[key] === other[key])
}

/** Rust 目录的单份 UI 投影；只有活动页面订阅视图提交。 */
export class CloudCatalogStore {
  private catalog: CloudCatalog | undefined
  private byId = new Map<string, CloudTitle>()
  private order: string[] = []
  private generation = 0
  private pending: Promise<void> | undefined
  private refreshing: Promise<void> | undefined
  private listeners = new Set<Listener>()
  private queued: DataXcloudCatalogUpdatedRendererEvent[] = []
  private dirty = false
  private syncing = false
  private queueOverflow = false
  private timer: ReturnType<typeof setTimeout> | undefined
  readonly metrics = { snapshots: 0, patches: 0, submissions: 0, duplicateContent: 0, resyncs: 0 }

  constructor(private readonly loader: CatalogLoader) {}

  get value(): CloudCatalog | undefined { return this.catalog }
  get subscriberCount(): number { return this.listeners.size }
  getTitle(id: string): CloudTitle | undefined { return this.byId.get(id) }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener)
    listener(this.catalog)
    return () => {
      this.listeners.delete(listener)
      if (!this.listeners.size) {
        clearTimeout(this.timer)
        this.timer = undefined
      }
    }
  }

  clear(): void {
    this.generation++
    this.pending = undefined
    this.refreshing = undefined
    this.catalog = undefined
    this.byId.clear()
    this.order = []
    this.queued = []
    this.dirty = false
    this.syncing = false
    this.queueOverflow = false
    this.notify(true)
  }

  ensure(): Promise<void> {
    if (this.pending)
      return this.pending
    const generation = this.generation
    const operation = (async () => {
      if (this.catalog) {
        const status = await this.loader.status()
        if (generation !== this.generation)
          return
        if (!this.dirty && status.scopeKey === this.catalog?.scopeKey && status.contentRevision && status.contentRevision === this.catalog?.contentRevision) {
          this.applyStatus(status)
          return
        }
      }
      await this.readSnapshot(generation)
    })().finally(() => {
      if (this.pending === operation)
        this.pending = undefined
    })
    this.pending = operation
    return operation
  }

  refresh(): Promise<void> {
    if (this.refreshing)
      return this.refreshing
    const generation = this.generation
    const operation = this.loader.refresh().then((catalog) => {
      if (generation === this.generation)
        this.applySnapshot(catalog)
    }).finally(() => {
      if (this.refreshing === operation)
        this.refreshing = undefined
    })
    this.refreshing = operation
    return operation
  }

  async summary(id: string): Promise<CloudTitle | undefined> {
    const generation = this.generation
    const value = await this.loader.summary(id)
    return generation === this.generation ? value.title ?? undefined : undefined
  }

  receive(event: DataXcloudCatalogUpdatedRendererEvent): void {
    if (this.catalog && event.scopeKey !== this.catalog.scopeKey)
      return
    if (!this.catalog || this.syncing) {
      if (this.queued.length < 160)
        this.queued.push(event)
      else
        this.queueOverflow = true
      return
    }
    if (this.isOlder(event))
      return
    if (!event.patch) {
      this.applySnapshot(event)
      return
    }
    if (event.contentRevision === this.catalog.contentRevision) {
      this.metrics.duplicateContent++
      this.applyStatus(event)
      return
    }
    if (this.dirty || event.patch.baseRevision !== this.catalog.contentRevision) {
      this.dirty = true
      if (this.listeners.size)
        void this.ensure().catch(() => {})
      return
    }
    const patch = event.patch
    for (const id of patch.removedProductIds)
      this.byId.delete(id)
    for (const title of patch.upserts)
      this.byId.set(title.productId, title)
    this.order = patch.order ?? this.order
    const titles = this.order.map(id => this.byId.get(id)).filter((title): title is CloudTitle => !!title)
    // 同版本状态更新保留 titles 对象；变更批次只替换实际更新的商品对象。
    this.catalog = { ...event, titles }
    this.metrics.patches++
    this.notify(!event.refreshing)
  }

  private async readSnapshot(generation: number, repair = true): Promise<void> {
    this.syncing = true
    let catalog: CloudCatalog
    try {
      catalog = await this.loader.snapshot()
    }
    finally {
      if (generation === this.generation)
        this.syncing = false
    }
    if (generation !== this.generation)
      return
    this.metrics.snapshots++
    const queued = this.queued
    const overflow = this.queueOverflow
    this.queued = []
    this.queueOverflow = false
    this.applySnapshot(catalog)
    for (const event of queued)
      this.receive(event)
    this.dirty ||= overflow
    if (this.dirty && this.listeners.size && repair) {
      // 首次订阅期间遇到 revision 缺口，在当前读完成后合并为一次同步。
      this.metrics.resyncs++
      await this.readSnapshot(generation, false)
    }
  }

  private isOlder(value: CloudCatalogStatus): boolean {
    return value.scopeKey === this.catalog?.scopeKey && this.catalog?.updatedAt !== undefined && (value.updatedAt ?? 0) < this.catalog.updatedAt
  }

  private applySnapshot(value: CloudCatalog): void {
    if (this.isOlder(value))
      return
    const same = value.scopeKey === this.catalog?.scopeKey && !!value.contentRevision && value.contentRevision === this.catalog?.contentRevision
    if (same) {
      this.metrics.duplicateContent++
      this.applyStatus(value)
    }
    else {
      this.byId = new Map(value.titles.map(title => [title.productId, title]))
      this.order = value.titles.map(title => title.productId)
      this.catalog = value
      this.notify(true)
    }
    this.dirty = false
  }

  private applyStatus(value: CloudCatalogStatus): void {
    if (!this.catalog || this.isOlder(value))
      return
    if (sameStatus(this.catalog, value))
      return
    const { titles } = this.catalog
    this.catalog = { ...value, titles }
    this.notify(!value.refreshing)
  }

  private notify(immediate: boolean): void {
    if (!this.listeners.size)
      return
    const submit = () => {
      clearTimeout(this.timer)
      this.timer = undefined
      this.metrics.submissions++
      this.listeners.forEach(listener => listener(this.catalog))
    }
    if (immediate)
      submit()
    else if (!this.timer)
      this.timer = setTimeout(submit, 250)
  }
}
