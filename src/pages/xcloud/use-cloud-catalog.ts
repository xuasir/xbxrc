import { events } from '@/services/events'
import { rpc } from '@/services/rpc'
import { CloudCatalogStore } from './catalog-store'

export const cloudCatalog = new CloudCatalogStore({
  snapshot: () => rpc.data.getXcloudTitles(),
  status: () => rpc.data.getCloudCatalogStatus(),
  refresh: () => rpc.data.refreshXcloudTitles(),
  summary: productId => rpc.data.getCloudTitleSummary({ productId }),
})
let initialized = false
export function useCloudCatalog() {
  if (!initialized) {
    initialized = true
    events.on('data.xcloudCatalogUpdated', event => cloudCatalog.receive(event))
    events.on('auth.sessionReady', () => cloudCatalog.clear())
    events.on('auth.stateChanged', (state) => {
      if (!state.isAuthenticated)
        cloudCatalog.clear()
    })
  }
  return cloudCatalog
}
