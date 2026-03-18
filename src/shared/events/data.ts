export const DATA_XCLOUD_CATALOG_UPDATED_CHANNEL = 'xbxrc:data:xcloud-catalog-updated'

export interface DataXcloudCatalogUpdatedRendererEvent {
  titles: Array<{
    id: string
    name: string
    productId: string
    titleId: string
    xboxTitleId?: number
    publisherName: string
    description: string
    tileImageUrl: string
    posterImageUrl: string
    categories: string[]
    supportedInputTypes: string[]
    hasEntitlement: boolean
    isRecentlyPlayed: boolean
    isNew: boolean
  }>
  cacheState: 'miss' | 'fresh' | 'stale'
  updatedAt?: number
  refreshing: boolean
  reason: string
}
