import { APP_PAGE_NAV_ORDER } from '@/navigation/spatial-nav.constants'

interface MotionRoute {
  name?: unknown
  meta: Record<string, unknown>
}
const DETAIL_DEPTH: Record<string, number> = { 'my-friend': 1, 'my-game': 1, 'console-games': 1, 'console-game': 2, 'controller-tool': 1, 'setting-stream-shortcuts': 1 }

interface PageMotion {
  direction: 'forward' | 'back' | 'fade'
  axis: 'auto' | 'x'
}

export function resolvePageMotion(to: MotionRoute, from: MotionRoute): PageMotion {
  const page = (route: MotionRoute) => (route.meta.navPage || route.name) as (typeof APP_PAGE_NAV_ORDER)[number]
  const next = APP_PAGE_NAV_ORDER.indexOf(page(to))
  const previous = APP_PAGE_NAV_ORDER.indexOf(page(from))
  if (next < 0 || previous < 0)
    return { direction: 'fade', axis: 'auto' }
  if (next !== previous)
    return { direction: next > previous ? 'forward' : 'back', axis: 'auto' }
  const depth = (route: MotionRoute) => DETAIL_DEPTH[String(route.name)] ?? 0
  // 同层切页跟随可见导航排列；详情进退保持横向层级关系。
  return { direction: depth(to) === depth(from) ? 'fade' : depth(to) > depth(from) ? 'forward' : 'back', axis: 'x' }
}
