import type { RouteRecordRaw } from 'vue-router'
import { createRouter, createWebHashHistory } from 'vue-router'
import { createAuthTraceOperationId, recordAuthTrace } from '../services/auth-trace'
import { rpc } from '../services/rpc'

const routes: RouteRecordRaw[] = [
  {
    path: '/',
    redirect: '/xhome',
  },
  {
    path: '/login',
    name: 'login',
    component: () => import('../pages/Login.vue'),
    meta: {
      guestOnly: true,
      layout: 'plain',
    },
  },
  {
    path: '/xhome',
    name: 'xhome',
    component: () => import('../pages/Home.vue'),
    meta: {
      requiresAuth: true,
      layout: 'shell',
      keepAlive: true,
    },
  },
  {
    path: '/xhome/stream/:targetId',
    name: 'xhome-stream',
    component: () => import('../pages/Stream.vue'),
    meta: {
      requiresAuth: true,
      layout: 'plain',
      streamTargetType: 'home',
    },
  },
  {
    path: '/xcloud/stream/:targetId',
    name: 'xcloud-stream',
    component: () => import('../pages/Stream.vue'),
    meta: {
      requiresAuth: true,
      layout: 'plain',
      streamTargetType: 'cloud',
    },
  },
  {
    path: '/xcloud',
    name: 'xcloud',
    component: () => import('../pages/XCloud.vue'),
    meta: {
      requiresAuth: true,
      layout: 'shell',
      keepAlive: true,
    },
  },
  { path: '/my', redirect: '/my/account' },
  { path: '/my/:section(account|achievements|statistics|album)', name: 'my', component: () => import('../pages/My.vue'), meta: { requiresAuth: true, layout: 'shell', navPage: 'my' } },
  { path: '/my/account/friends/:targetXuid', name: 'my-friend', component: () => import('../pages/FriendDetail.vue'), meta: { requiresAuth: true, layout: 'shell', navPage: 'my' } },
  { path: '/my/achievements/:xboxTitleId', name: 'my-game', component: () => import('../pages/GameDetail.vue'), meta: { requiresAuth: true, layout: 'shell', navPage: 'my' } },
  { path: '/xhome/:consoleKey/games', name: 'console-games', component: () => import('../pages/ConsoleLibrary.vue'), meta: { requiresAuth: true, layout: 'shell', navPage: 'xhome' } },
  { path: '/xhome/:consoleKey/games/:installedItemKey', name: 'console-game', component: () => import('../pages/GameDetail.vue'), meta: { requiresAuth: true, layout: 'shell', navPage: 'xhome' } },
  {
    path: '/setting',
    name: 'setting',
    component: () => import('../pages/Setting.vue'),
    meta: {
      requiresAuth: true,
      layout: 'shell',
      keepAlive: true,
    },
  },
  {
    path: '/controller',
    name: 'controller',
    component: () => import('../pages/Controller.vue'),
    meta: {
      requiresAuth: true,
      layout: 'shell',
      keepAlive: true,
    },
  },
]

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
})

async function resolveAuthState(): Promise<Awaited<ReturnType<typeof rpc.auth.getState>>> {
  return await rpc.auth.getState()
}

const navigationTraces = new WeakMap<object, { operationId: string, startedAt: number }>()

router.beforeEach(async (to, from) => {
  const operationId = createAuthTraceOperationId()
  navigationTraces.set(to, { operationId, startedAt: performance.now() })
  const context = { operationId, source: 'router' as const, to, from }
  recordAuthTrace('routeNavigationStarted', context)
  const requiresAuth = to.matched.some(record => record.meta.requiresAuth === true)
  const guestOnly = to.matched.some(record => record.meta.guestOnly === true)

  if (!requiresAuth && !guestOnly) {
    recordAuthTrace('routeGuardResolved', { ...context, outcome: 'allowed' })
    return true
  }

  let authState: Awaited<ReturnType<typeof rpc.auth.getState>>
  try {
    authState = await resolveAuthState()
    recordAuthTrace('routeAuthStateObserved', { ...context, state: authState })
  }
  catch (error) {
    recordAuthTrace('routeGuardResolved', { ...context, error, outcome: requiresAuth ? 'redirect' : 'allowed' })
    if (requiresAuth) {
      return {
        name: 'login',
        query: {
          redirect: to.fullPath,
        },
      }
    }
    return true
  }

  if (requiresAuth && !authState.isAuthenticated) {
    recordAuthTrace('routeGuardResolved', { ...context, outcome: 'redirect' })
    return {
      name: 'login',
      query: {
        redirect: to.fullPath,
      },
    }
  }

  if (guestOnly && authState.isAuthenticated) {
    recordAuthTrace('routeGuardResolved', { ...context, outcome: 'redirect' })
    const redirectTarget = typeof to.query.redirect === 'string' ? to.query.redirect : '/xhome'
    return redirectTarget
  }

  recordAuthTrace('routeGuardResolved', { ...context, outcome: 'allowed' })
  return true
})

router.afterEach((to, from, failure) => {
  const operation = navigationTraces.get(to)
  recordAuthTrace('routeNavigationCompleted', { source: 'router', to, from, operationId: operation?.operationId, durationMs: operation ? Math.round(performance.now() - operation.startedAt) : undefined, outcome: failure ? 'failed' : 'success', navigationFailureType: failure?.type })
  navigationTraces.delete(to)
})

router.onError((error, to, from) => {
  const operation = navigationTraces.get(to)
  recordAuthTrace('routeNavigationFailed', { source: 'router', to, from, operationId: operation?.operationId, durationMs: operation ? Math.round(performance.now() - operation.startedAt) : undefined, error })
  navigationTraces.delete(to)
})
