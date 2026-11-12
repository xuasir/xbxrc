import type { CloudCatalog, CloudCatalogPatch } from '../rpc/cloud'

export const DATA_XCLOUD_CATALOG_UPDATED_CHANNEL = 'xbxrc:data:xcloud-catalog-updated'
export const DATA_HOSTS_CHANGED_CHANNEL = 'xbxrc:data:hosts-changed'
export interface DataHostsChangedRendererEvent {
  scopeKey: string
}
export interface DataXcloudCatalogUpdatedRendererEvent extends CloudCatalog {
  scopeKey: string
  reason: string
  patch?: CloudCatalogPatch
}
