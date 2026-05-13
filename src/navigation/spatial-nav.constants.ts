export const SPATIAL_NAV_SCOPE_IDS = {
  appShell: 'app.shell',
  userMenu: 'user.menu',
  gamepadMenu: 'gamepad.menu',
  login: 'login.scope',
  settingSingleSelect: 'setting.single-select',
  streamPage: 'stream.page',
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
  gamepadMenu: {
    close: 'gamepad-menu.close',
  },
  settingTabs: {
    general: 'setting.tabs.general',
    streamingExperience: 'setting.tabs.streamingExperience',
    connectionHost: 'setting.tabs.connectionHost',
    inputDevices: 'setting.tabs.inputDevices',
    advancedDiagnostics: 'setting.tabs.advancedDiagnostics',
  },
  pagePrimary: {
    xhome: 'xhome.primary',
    xcloud: 'xcloud.primary',
    setting: 'setting.tabs.general',
  },
  login: {
    signIn: 'login.sign-in',
  },
  streamPage: {
    /** 关闭弹层后承接焦点，避免焦点回到「菜单」按钮时被仍按住的 A 再次点开菜单 */
    focusSink: 'stream.page.focus-sink',
    menu: 'stream.action.menu',
    diagnostics: 'stream.action.diagnostics',
    fullscreen: 'stream.action.fullscreen',
    sendText: 'stream.action.send-text',
    powerOff: 'stream.action.power-off',
    exit: 'stream.action.exit',
    retry: 'stream.action.retry',
    back: 'stream.action.back',
  },
} as const

export type TopNavNodeKey = keyof typeof SPATIAL_NAV_NODE_IDS.topNav
export type SettingTabKey = keyof typeof SPATIAL_NAV_NODE_IDS.settingTabs
export type AppPageRouteName = keyof typeof SPATIAL_NAV_NODE_IDS.pagePrimary
