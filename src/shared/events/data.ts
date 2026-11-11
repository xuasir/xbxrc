import type { CloudCatalog } from '../rpc/cloud'

export const DATA_XCLOUD_CATALOG_UPDATED_CHANNEL = 'xbxrc:data:xcloud-catalog-updated'
export interface DataXcloudCatalogUpdatedRendererEvent extends CloudCatalog {
  scopeKey: string
  reason: string
}
