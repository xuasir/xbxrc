export interface CloudTitle {
  id: string
  name: string
  productId: string
  titleId: string
  xboxTitleId?: number | null
  publisherName: string
  description: string
  tileImageUrl: string
  posterImageUrl: string
  categories: string[]
  supportedInputTypes: string[]
  hasEntitlement: boolean
  entitlementKnown?: boolean
  aliases?: string[]
  isRecentlyPlayed: boolean
  isNew: boolean
  recentRank?: number | null
  newRank?: number | null
  popularRank?: number | null
  leavingRank?: number | null
}
export interface CloudCatalog {
  scopeKey?: string
  titles: CloudTitle[]
  cacheState: 'miss' | 'fresh' | 'stale'
  updatedAt?: number
  refreshing: boolean
  access?: string
  sources?: Record<string, string>
  error?: string | null
}
export interface CloudPreferences {
  favorites: CloudTitle[]
  pinned: string[]
  section: string
  category: string
  sort: string
}
export interface CloudDetail {
  detail: {
    product: { productId: string, name: string, publisherName: string, description: string, tileImageUrl: string, posterImageUrl: string, heroImageUrl: string, categories: string[] }
    developer: string
    releaseDate: string
    rating: string
    attributes: string[]
    screenshots: string[]
    platforms: string[]
    languages: Array<{ locale: string, interface: number | null, audio: number | null, subtitles: number | null }>
    accessibility: string[]
  }
  updatedAt: number
  stale: boolean
}
export interface CloudWait { seconds: number | null, updatedAt: number }
