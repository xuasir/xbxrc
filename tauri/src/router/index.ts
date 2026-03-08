import { createRouter, createWebHashHistory, type RouteRecordRaw } from 'vue-router'
import { rpc } from '../services/rpc'

const routes: RouteRecordRaw[] = [
  {
    path: '/',
    redirect: '/xhome'
  },
  {
    path: '/login',
    name: 'login',
    component: () => import('../pages/Login.vue'),
    meta: {
      guestOnly: true,
      layout: 'plain'
    }
  },
  {
    path: '/xhome',
    name: 'xhome',
    component: () => import('../pages/Home.vue'),
    meta: {
      requiresAuth: true,
      layout: 'shell',
      keepAlive: true
    }
  },
  {
    path: '/xhome/stream/:targetId',
    name: 'xhome-stream',
    component: () => import('../pages/XStream.vue'),
    meta: {
      requiresAuth: true,
      layout: 'plain',
      streamTargetType: 'home'
    }
  },
  {
    path: '/xcloud/stream/:targetId',
    name: 'xcloud-stream',
    component: () => import('../pages/XStream.vue'),
    meta: {
      requiresAuth: true,
      layout: 'plain',
      streamTargetType: 'cloud'
    }
  },
  {
    path: '/xcloud',
    name: 'xcloud',
    component: () => import('../pages/XCloud.vue'),
    meta: {
      requiresAuth: true,
      layout: 'shell',
      keepAlive: true
    }
  },
  {
    path: '/setting',
    name: 'setting',
    component: () => import('../pages/Setting.vue'),
    meta: {
      requiresAuth: true,
      layout: 'shell',
      keepAlive: true
    }
  }
]

export const router = createRouter({
  history: createWebHashHistory(),
  routes
})

async function resolveAuthState(): Promise<Awaited<ReturnType<typeof rpc.auth.getState>>> {
  const currentState = await rpc.auth.getState()
  if (currentState.isAuthenticated || currentState.isAuthenticating) {
    return currentState
  }

  // 在导航守卫里补一次静默认证，减少误跳登录页
  await rpc.auth.checkAuthentication()
  return await rpc.auth.getState()
}

router.beforeEach(async (to) => {
  const requiresAuth = to.matched.some((record) => record.meta.requiresAuth === true)
  const guestOnly = to.matched.some((record) => record.meta.guestOnly === true)

  if (!requiresAuth && !guestOnly) {
    return true
  }

  let authState: Awaited<ReturnType<typeof rpc.auth.getState>>
  try {
    authState = await resolveAuthState()
  } catch {
    if (requiresAuth) {
      return {
        name: 'login',
        query: {
          redirect: to.fullPath
        }
      }
    }
    return true
  }

  if (requiresAuth && !authState.isAuthenticated) {
    return {
      name: 'login',
      query: {
        redirect: to.fullPath
      }
    }
  }

  if (guestOnly && authState.isAuthenticated) {
    const redirectTarget = typeof to.query.redirect === 'string' ? to.query.redirect : '/xhome'
    return redirectTarget
  }

  return true
})
