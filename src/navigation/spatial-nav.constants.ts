import type { TabLevel } from '@spatial-navigation/runtime'

export const SPATIAL_NAV_SCOPE_IDS = {
  appShell: 'app.shell',
  userMenu: 'user.menu',
  login: 'login.scope',
  settingSingleSelect: 'setting.single-select',
  streamPage: 'stream.page',
} as const

export const SPATIAL_NAV_TAB_LEVELS: Record<'primary' | 'secondary', TabLevel> = {
  primary: 'primary',
  secondary: 'secondary',
} as const

export const SPATIAL_NAV_NODE_IDS = {
  topNav: {
    brand: 'top-nav.brand',
    xhome: 'top-nav.xhome',
    xcloud: 'top-nav.xcloud',
    setting: 'top-nav.setting',
    controller: 'top-nav.controller',
    profile: 'top-nav.profile',
  },
  userMenu: {
    idle: 'user-menu.idle',
    info: 'user-menu.info',
    status: 'user-menu.status',
    logout: 'user-menu.logout',
  },
  settingTabs: {
    app: 'setting.tabs.app',
    streaming: 'setting.tabs.streaming',
    host: 'setting.tabs.host',
    xcloud: 'setting.tabs.xcloud',
    input: 'setting.tabs.input',
  },
  pagePrimary: {
    xhome: 'xhome.primary',
    xcloud: 'xcloud.primary',
    setting: 'setting.tabs.app',
  },
  login: {
    signIn: 'login.sign-in',
  },
  streamPage: {
    menu: 'stream.action.menu',
    fullscreen: 'stream.action.fullscreen',
    sendText: 'stream.action.send-text',
    powerOff: 'stream.action.power-off',
    exit: 'stream.action.exit',
    retry: 'stream.action.retry',
    back: 'stream.action.back',
  },
} as const

export const SPATIAL_NAV_PRIMARY_TAB_ORDER = {
  xhome: 0,
  xcloud: 1,
  setting: 2,
} as const

export const SPATIAL_NAV_KEYBOARD_SHORTCUTS = {
  primaryPrev: 'q',
  primaryNext: 'e',
  secondaryPrev: 'z',
  secondaryNext: 'c',
} as const

export const SPATIAL_NAV_RUNTIME_EVENTS = {
  tabNavAction: 'spatial-nav:tab-nav-action',
} as const

export type TopNavNodeKey = keyof typeof SPATIAL_NAV_NODE_IDS.topNav
export type SettingTabKey = keyof typeof SPATIAL_NAV_NODE_IDS.settingTabs
export type AppPageRouteName = keyof typeof SPATIAL_NAV_NODE_IDS.pagePrimary
