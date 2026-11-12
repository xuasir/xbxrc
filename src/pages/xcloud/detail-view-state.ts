import type { CloudDetail } from '@/shared/rpc/cloud'

interface DetailViewState { detail?: CloudDetail, expanded: boolean, languages: boolean, scrollTop: number, screenshotScroll: number }
const views = new Map<string, DetailViewState>()
export function saveCloudDetailView(key: string, value: DetailViewState): void {
  views.delete(key)
  views.set(key, value)
  while (views.size > 30)
    views.delete(views.keys().next().value!)
}
export function cloudDetailView(key: string): DetailViewState | undefined {
  const value = views.get(key)
  if (value) {
    views.delete(key)
    views.set(key, value)
  }
  return value
}
