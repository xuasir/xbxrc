import Store from 'electron-store'
import type { DataSessionContext, DataXcloudTitleSummary } from '../../domain/types'
import { getMainStore, STORE_KEYS } from '../../../../store'

interface XcloudCatalogImage {
  URL?: string
}

interface XcloudCatalogProduct {
  ProductTitle?: string
  PublisherName?: string
  ProductDescription?: string
  Image_Tile?: XcloudCatalogImage
  Image_Poster?: XcloudCatalogImage
  LocalizedCategories?: unknown[]
  Categories?: unknown[]
  XCloudTitleId?: string
  XboxTitleId?: string | number
}

interface XcloudStreamingTitle {
  titleId?: string
  details?: {
    productId?: string
    xboxTitleId?: number
    hasEntitlement?: boolean
    supportedInputTypes?: unknown[]
  }
}

interface XcloudTitlesResponse {
  results?: unknown[]
}

interface XcloudRecentTitlesResponse {
  results?: Array<{
    details?: {
      productId?: string
    }
  }>
}

interface XcloudNewestTitleEntry {
  id?: string
}

interface XcloudCatalogProductsResponse {
  Products?: Record<string, XcloudCatalogProduct>
}

interface ResolvedXcloudRegion {
  host: string
  bearerToken: string
}

interface FetchJsonOptions {
  expectedStatus?: number[]
  timeoutMs?: number
}

interface XcloudTitlesCachePayload {
  updatedAt: number
  titles: DataXcloudTitleSummary[]
}

const XCLOUD_TITLES_CACHE_TTL_MS = 10 * 60 * 1000
const XCLOUD_TITLES_CACHE_STALE_MAX_MS = 24 * 60 * 60 * 1000

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function asNonEmptyString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() !== '' ? value.trim() : undefined
}

function normalizeProductId(value: unknown): string | undefined {
  const productId = asNonEmptyString(value)
  return productId?.toUpperCase()
}

function resolveImageUrl(value: unknown): string {
  const imageUrl = asNonEmptyString(value)
  if (imageUrl === undefined) {
    return ''
  }
  if (imageUrl.startsWith('//')) {
    return `https:${imageUrl}`
  }
  return imageUrl
}

function uniqueStrings(values: Array<string | undefined>): string[] {
  return [...new Set(values.filter((value): value is string => value !== undefined))]
}

function resolveXboxTitleId(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value
  }
  if (typeof value === 'string') {
    const parsedValue = Number.parseInt(value, 10)
    return Number.isFinite(parsedValue) ? parsedValue : undefined
  }
  return undefined
}

function chunkValues(values: string[], size: number): string[][] {
  const chunks: string[][] = []
  for (let index = 0; index < values.length; index += size) {
    chunks.push(values.slice(index, index + size))
  }
  return chunks
}

function isDataXcloudTitleSummary(value: DataXcloudTitleSummary | null): value is DataXcloudTitleSummary {
  return value !== null
}

function extractStreamingTitles(rawResponse: unknown): XcloudStreamingTitle[] {
  if (!isRecord(rawResponse) || !Array.isArray(rawResponse.results)) {
    return []
  }

  return rawResponse.results.filter((item): item is XcloudStreamingTitle => {
    if (!isRecord(item)) {
      return false
    }
    if (!isRecord(item.details)) {
      return false
    }
    return normalizeProductId(item.details.productId) !== undefined
  })
}

function extractRecentProductIds(rawResponse: unknown): string[] {
  if (!isRecord(rawResponse) || !Array.isArray(rawResponse.results)) {
    return []
  }

  return uniqueStrings(
    rawResponse.results.map((item) => {
      if (!isRecord(item) || !isRecord(item.details)) {
        return undefined
      }
      return normalizeProductId(item.details.productId)
    })
  )
}

function extractNewestProductIds(rawResponse: unknown): string[] {
  if (!Array.isArray(rawResponse)) {
    return []
  }

  return uniqueStrings(
    rawResponse.map((item) => {
      if (!isRecord(item)) {
        return undefined
      }
      return normalizeProductId(item.id)
    })
  )
}

function extractCatalogProducts(rawResponse: unknown): Record<string, XcloudCatalogProduct> {
  if (!isRecord(rawResponse) || !isRecord(rawResponse.Products)) {
    return {}
  }

  const products: Record<string, XcloudCatalogProduct> = {}
  Object.entries(rawResponse.Products).forEach(([productId, value]) => {
    if (isRecord(value)) {
      products[productId.toUpperCase()] = value as XcloudCatalogProduct
    }
  })
  return products
}

function resolveCatalogLanguage(): string {
  const locale = Intl.DateTimeFormat().resolvedOptions().locale.toLowerCase()
  return locale.startsWith('zh') ? 'zh-TW' : 'en-US'
}

function resolveXcloudRegion(session: DataSessionContext): ResolvedXcloudRegion | null {
  const tokenData = session.streamingTokens.xCloudToken?.data
  const bearerToken = asNonEmptyString(tokenData?.gsToken)
  const regions = tokenData?.offeringSettings?.regions
  const region =
    regions?.find((item) => item?.isDefault === true) ??
    regions?.find((item) => asNonEmptyString(item?.baseUri) !== undefined)

  const baseUri = asNonEmptyString(region?.baseUri)
  if (bearerToken === undefined || baseUri === undefined) {
    return null
  }

  try {
    const parsedUrl = new URL(baseUri)
    return {
      host: parsedUrl.host,
      bearerToken
    }
  } catch {
    return null
  }
}

async function fetchJson<T>(
  url: string,
  init?: RequestInit,
  options: FetchJsonOptions = {}
): Promise<T> {
  const expectedStatus = options.expectedStatus ?? [200]
  const timeoutMs = options.timeoutMs ?? 15000
  const response = await fetch(url, {
    ...init,
    signal: AbortSignal.timeout(timeoutMs)
  })

  if (!expectedStatus.includes(response.status)) {
    const errorBody = await response.text()
    throw new Error(`HTTP ${response.status} for ${url}: ${errorBody}`)
  }

  if (response.status === 204) {
    return {} as T
  }

  return (await response.json()) as T
}

async function fetchJsonOrFallback<T>(
  url: string,
  fallback: T,
  init?: RequestInit,
  options?: FetchJsonOptions
): Promise<T> {
  try {
    return await fetchJson<T>(url, init, options)
  } catch (error) {
    // 目录接口网络波动时允许使用缓存/默认值兜底，测试阶段降级为 debug 避免噪声误导。
    console.debug(`[Data] fallback to cached/default payload for ${url}:`, error)
    return fallback
  }
}

/**
 * 云游戏目录服务
 * - 基于 xCloud 标题接口 + Game Pass catalog 聚合页面所需元数据
 */
export class XcloudService {
  private readonly store: Store
  private inMemoryCache: XcloudTitlesCachePayload | null = null
  private refreshPromise: Promise<DataXcloudTitleSummary[]> | null = null

  constructor(store?: Store) {
    this.store = store ?? getMainStore()
  }

  async getTitles(session: DataSessionContext): Promise<DataXcloudTitleSummary[]> {
    const cachedPayload = this.getCachedTitles()
    if (cachedPayload !== null) {
      if (Date.now() - cachedPayload.updatedAt > XCLOUD_TITLES_CACHE_TTL_MS) {
        void this.refreshTitlesInBackground(session)
      }
      return cachedPayload.titles
    }

    return await this.fetchAndCacheTitles(session)
  }

  private getCachedTitles(): XcloudTitlesCachePayload | null {
    if (this.inMemoryCache !== null && this.isCacheUsable(this.inMemoryCache)) {
      return this.inMemoryCache
    }

    const cachedValue = this.store.get(STORE_KEYS.DATA.XCLOUD_TITLES_CACHE, null)
    if (!isRecord(cachedValue) || typeof cachedValue.updatedAt !== 'number') {
      return null
    }
    if (!Array.isArray(cachedValue.titles)) {
      return null
    }

    const titles = cachedValue.titles.filter((item): item is DataXcloudTitleSummary => {
      return isRecord(item) && typeof item.id === 'string' && typeof item.titleId === 'string'
    })
    if (titles.length === 0) {
      return null
    }

    const payload: XcloudTitlesCachePayload = {
      updatedAt: cachedValue.updatedAt,
      titles
    }
    if (!this.isCacheUsable(payload)) {
      return null
    }

    this.inMemoryCache = payload
    return payload
  }

  private isCacheUsable(payload: XcloudTitlesCachePayload): boolean {
    return Date.now() - payload.updatedAt <= XCLOUD_TITLES_CACHE_STALE_MAX_MS
  }

  private async refreshTitlesInBackground(session: DataSessionContext): Promise<void> {
    if (this.refreshPromise !== null) {
      return
    }

    this.refreshPromise = this.fetchAndCacheTitles(session)
    try {
      await this.refreshPromise
    } catch {
      // 后台刷新失败时保留旧缓存，不阻断当前页面展示。
    } finally {
      this.refreshPromise = null
    }
  }

  private async fetchAndCacheTitles(session: DataSessionContext): Promise<DataXcloudTitleSummary[]> {
    const region = resolveXcloudRegion(session)
    if (region === null) {
      return []
    }

    try {
      const [streamingTitlesResponse, recentTitlesResponse, newestTitlesResponse] = await Promise.all([
        fetchJson<XcloudTitlesResponse>(`https://${region.host}/v2/titles`, {
          headers: {
            Authorization: `Bearer ${region.bearerToken}`,
            'Content-Type': 'application/json'
          }
        }, {
          timeoutMs: 25000
        }),
        fetchJsonOrFallback<XcloudRecentTitlesResponse>(
          `https://${region.host}/v2/titles/mru?mr=25`,
          { results: [] },
          {
            headers: {
              Authorization: `Bearer ${region.bearerToken}`,
              'Content-Type': 'application/json'
            }
          },
          {
            timeoutMs: 12000
          }
        ),
        fetchJsonOrFallback<XcloudNewestTitleEntry[]>(
          'https://catalog.gamepass.com/sigls/v2?id=f13cf6b4-57e6-4459-89df-6aec18cf0538&market=US&language=en-US',
          [],
          undefined,
          {
            timeoutMs: 10000
          }
        )
      ])

      const streamingTitles = extractStreamingTitles(streamingTitlesResponse)
      const liveTitleMap = new Map<string, XcloudStreamingTitle>()
      streamingTitles.forEach((title) => {
        const productId = normalizeProductId(title.details?.productId)
        if (productId !== undefined) {
          liveTitleMap.set(productId, title)
        }
      })

      const productIds = uniqueStrings(
        streamingTitles.map((title) => normalizeProductId(title.details?.productId))
      )

      const catalogProducts = await this.loadCatalogProducts(productIds)
      const recentProductIds = new Set(extractRecentProductIds(recentTitlesResponse))
      const newestProductIds = new Set(extractNewestProductIds(newestTitlesResponse))

      const resolvedTitles: Array<DataXcloudTitleSummary | null> = productIds.map((productId) => {
          const liveTitle = liveTitleMap.get(productId)
          const catalogProduct = catalogProducts[productId]
          const resolvedTitleId =
            asNonEmptyString(liveTitle?.titleId) ?? asNonEmptyString(catalogProduct?.XCloudTitleId) ?? ''
          const resolvedName =
            asNonEmptyString(catalogProduct?.ProductTitle) ?? asNonEmptyString(liveTitle?.titleId) ?? productId

          // 目录标题至少需要可读名称和可用于后续串流的 titleId。
          if (resolvedName === '' || resolvedTitleId === '') {
            return null
          }

          const supportedInputTypes = Array.isArray(liveTitle?.details?.supportedInputTypes)
            ? uniqueStrings(
                liveTitle?.details?.supportedInputTypes.map((item) => asNonEmptyString(item))
              )
            : []

          const categoriesSource = Array.isArray(catalogProduct?.LocalizedCategories)
            ? catalogProduct.LocalizedCategories
            : Array.isArray(catalogProduct?.Categories)
              ? catalogProduct.Categories
              : []

          return {
            id: productId,
            productId,
            titleId: resolvedTitleId,
            xboxTitleId:
              resolveXboxTitleId(liveTitle?.details?.xboxTitleId) ??
              resolveXboxTitleId(catalogProduct?.XboxTitleId),
            name: resolvedName,
            publisherName: asNonEmptyString(catalogProduct?.PublisherName) ?? '',
            description: asNonEmptyString(catalogProduct?.ProductDescription) ?? '',
            tileImageUrl: resolveImageUrl(catalogProduct?.Image_Tile?.URL),
            posterImageUrl: resolveImageUrl(catalogProduct?.Image_Poster?.URL),
            categories: uniqueStrings(categoriesSource.map((item) => asNonEmptyString(item))),
            supportedInputTypes,
            hasEntitlement: liveTitle?.details?.hasEntitlement !== false,
            isRecentlyPlayed: recentProductIds.has(productId),
            isNew: newestProductIds.has(productId)
          }
        })
      const titles = resolvedTitles
        .filter(isDataXcloudTitleSummary)
        .sort((left, right) => left.name.localeCompare(right.name))
      this.setCachedTitles(titles)
      return titles
    } catch (error) {
      console.warn('[Data] load xcloud titles failed:', error)
      return this.getCachedTitles()?.titles ?? []
    }
  }

  private setCachedTitles(titles: DataXcloudTitleSummary[]): void {
    if (titles.length === 0) {
      return
    }

    const payload: XcloudTitlesCachePayload = {
      updatedAt: Date.now(),
      titles
    }
    this.inMemoryCache = payload
    this.store.set(STORE_KEYS.DATA.XCLOUD_TITLES_CACHE, payload)
  }

  private async loadCatalogProducts(
    productIds: string[]
  ): Promise<Record<string, XcloudCatalogProduct>> {
    if (productIds.length === 0) {
      return {}
    }

    const catalogLanguage = resolveCatalogLanguage()
    const productChunks = chunkValues(productIds, 75)
    const responseMap: Record<string, XcloudCatalogProduct> = {}

    for (const chunk of productChunks) {
      const response = await fetchJsonOrFallback<XcloudCatalogProductsResponse>(
          `https://catalog.gamepass.com/v3/products?market=US&language=${catalogLanguage}&hydration=RemoteLowJade0`,
          { Products: {} },
          {
            method: 'POST',
            headers: {
              Accept: 'application/json',
              'Content-Type': 'application/json',
              'ms-cv': '0',
              'calling-app-name': 'Xbox Cloud Gaming Web',
              'calling-app-version': '24.17.63'
            },
            body: JSON.stringify({
              Products: chunk
            })
          },
          {
            timeoutMs: 18000
          }
        )

      Object.assign(responseMap, extractCatalogProducts(response))
    }

    return responseMap
  }
}
