import '../2026-11-09-gamepad-focus-action-completion/project-fixture.js'
import { nextTick } from 'vue'
import { finishUiTransitions, hasUiTransitions } from '/src/app/ui-motion.ts'
import { navigationEngine } from '/src/navigation/core/engine.ts'
import { isFocusTarget } from '/src/navigation/core/focus-target.ts'
import { inputDispatcher, NavigationIntent } from '/src/navigation/core/input.ts'
import { requestPageFocus } from '/src/navigation/core/usePageFocus.ts'
import { router } from '/src/router/index.ts'
import { inputController } from '/src/shared/gamepad/runtime.ts'

// 只在此离线入口模拟系统偏好；实际媒体元素、WAAPI、Vue 与导航全部使用生产实现。
const nativeMatchMedia = window.matchMedia.bind(window)
const reducedPreference = Object.assign(new EventTarget(), { matches: false, media: '(prefers-reduced-motion: reduce)' })
window.matchMedia = (query: string) => query.includes('prefers-reduced-motion') ? reducedPreference as MediaQueryList : nativeMatchMedia(query)
const errors: string[] = []
window.addEventListener('error', event => errors.push(event.message))
window.addEventListener('unhandledrejection', event => errors.push(String(event.reason)))
await import('/src/main.ts')

const output = document.getElementById('motion-results')!
const button = document.getElementById('motion-run') as HTMLButtonElement
const records: { name: string, passed: boolean, detail: unknown }[] = []
const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
async function settle(ms = 320) { await nextTick(); await wait(ms); await frame() }
async function until(test: () => boolean, label: string, timeout = 4000) {
  const start = performance.now()
  while (!test()) {
    if (performance.now() - start > timeout) throw new Error(`等待超时：${label}`)
    await wait(20)
  }
}
function check(name: string, passed: boolean, detail: unknown) {
  records.push({ name, passed, detail })
  output.textContent = JSON.stringify({ passed: records.filter(item => item.passed).length, failed: records.filter(item => !item.passed).length, records, errors }, null, 2)
  if (!passed) throw new Error(name)
}
function node(id: string): HTMLElement {
  const element = document.getElementById(id)
  if (!element) throw new Error(`节点缺失：${id}`)
  return element
}
const ring = () => document.querySelector<HTMLElement>('.focus-flow-ring')!
function snapshot() {
  const outline = document.querySelector<HTMLElement>('[data-ui-focus-flow]')
  const box = outline?.getBoundingClientRect()
  const ringBox = ring().getBoundingClientRect()
  return {
    route: router.currentRoute.value.name,
    motion: document.documentElement.dataset.uiMotion,
    focus: navigationEngine.getCurrentFocus()?.id,
    owner: outline?.id,
    ringCount: document.querySelectorAll('.focus-flow-ring').length,
    owners: document.querySelectorAll('[data-ui-focus-flow]').length,
    hidden: ring().hidden,
    ring: ringBox.toJSON(),
    target: box?.toJSON(),
    maxDelta: box ? Math.max(Math.abs(ringBox.left - box.left + 4), Math.abs(ringBox.top - box.top + 4), Math.abs(ringBox.width - box.width - 8), Math.abs(ringBox.height - box.height - 8)) : null,
    padding: getComputedStyle(ring()).padding,
    angle: getComputedStyle(ring()).getPropertyValue('--ui-focus-angle'),
    themeColorMatches: !outline || getComputedStyle(ring()).getPropertyValue('--ui-focus-ring-start') === getComputedStyle(outline).getPropertyValue('--ui-focus-ring-start'),
    ringAnimations: ring().getAnimations().length,
    transitions: hasUiTransitions(),
    activePages: document.querySelectorAll('[data-nav-page-active="true"]').length,
    ambient: getComputedStyle(document.querySelector('.app-shell')!, '::after').animationPlayState,
    viewport: { width: innerWidth, height: innerHeight },
    theme: document.documentElement.dataset.theme,
  }
}

button.onclick = async () => {
  button.disabled = true
  records.length = 0
  const nativeAnimate = Element.prototype.animate
  const animations: { target: Element, frames: Keyframe[] | PropertyIndexedKeyframes | null, duration: number | undefined, delay: number | undefined }[] = []
  Element.prototype.animate = function (frames, options) {
    const settings = typeof options === 'number' ? { duration: options } : options
    animations.push({ target: this, frames, duration: Number(settings?.duration), delay: settings?.delay })
    return nativeAnimate.call(this, frames, options)
  }
  try {
    inputController.setBackground(false)
    await router.push('/xhome')
    await until(() => !!document.getElementById('xhome.primary'), '首页加载')
    await settle()
    navigationEngine.focusElement(node('top-nav.xhome'), false)
    await settle(120)
    let state = snapshot()
    check('单例外环、2px 均匀边框与四边对齐', state.ringCount === 1 && state.owners === 1 && state.maxDelta! < .6 && state.padding === '2px', state)
    const angleBefore = Number.parseFloat(state.angle)
    navigationEngine.focusElement(node('top-nav.xcloud'), false)
    await frame()
    await frame()
    const first = animations.filter(item => item.target === ring()).at(-1)!
    check('邻近焦点 220ms 整体飞行', first.duration === 220 && !!(first.frames as Keyframe[])[0].left, { duration: first.duration, frames: first.frames })
    await wait(35)
    const midflight = ring().getBoundingClientRect().toJSON()
    navigationEngine.focusElement(node('top-nav.my'), false)
    await frame()
    const redirected = animations.filter(item => item.target === ring()).at(-1)!
    check('连按沿当前视觉位置重定向，逻辑目标即时更新', navigationEngine.getCurrentFocus()?.id === 'top-nav.my' && redirected.duration === 220 && Number.parseFloat(String((redirected.frames as Keyframe[])[0].left)) >= midflight.left - 1, { midflight, frames: redirected.frames })
    await settle(260)
    state = snapshot()
    check('飞行收稳，角度连续且停稳后静止', state.maxDelta! < .6 && Number.parseFloat(state.angle) > angleBefore && state.ringAnimations === 0, state)
    const stoppedAngle = state.angle
    await wait(120)
    check('焦点静止阶段释放持续动画', snapshot().angle === stoppedAngle && ring().getAnimations().length === 0, snapshot())

    navigationEngine.focusElement(node('xhome.primary'), false)
    await settle(120)
    navigationEngine.focusElement(node('xhome.installed-games'), false)
    await frame()
    await frame()
    const resized = animations.filter(item => item.target === ring()).at(-1)!
    check('邻近不同尺寸控件同步过渡外环宽高', resized.duration === 220 && (resized.frames as Keyframe[])[0].width !== (resized.frames as Keyframe[])[1].width, { frames: resized.frames })
    await settle(260)
    check('尺寸飞行收稳后维持四边对齐', snapshot().maxDelta! < .6 && snapshot().padding === '2px', snapshot())
    navigationEngine.focusElement(node('top-nav.my'), false)
    await settle(120)

    navigationEngine.focusElement(node('top-nav.controller'), false)
    await frame()
    inputDispatcher.dispatch(NavigationIntent.Action)
    await until(() => router.currentRoute.value.name === 'controller', '飞行期间确认')
    check('飞行期间 A 即时确认当前逻辑目标', router.currentRoute.value.name === 'controller', { route: router.currentRoute.value.name })
    await settle()
    const controller = document.querySelector('.controller-page')!
    await router.push('/xhome')
    await settle()
    const home = document.querySelector('.home-page')!
    const start = animations.length
    requestPageFocus()
    await router.push('/setting')
    await frame()
    const newPage = document.querySelector('.setting-page')!
    check('并行切页：新页激活，旧页 inert 退出候选', !!newPage && home.hasAttribute('inert') && !isFocusTarget(home.querySelector('[data-focusable="true"]') as HTMLElement) && document.querySelectorAll('[data-nav-page-active="true"]').length === 1, { oldInert: home.hasAttribute('inert'), activePages: document.querySelectorAll('[data-nav-page-active="true"]').length })
    const pageAnimation = animations.slice(start).find(item => item.target === newPage && (item.frames as Keyframe[])[0].transform)
    const pageMenuDirection = getComputedStyle(document.querySelector('.top-nav__dock')!).flexDirection
    const pageAxis = pageMenuDirection.startsWith('column') ? 'Y' : 'X'
    check('一级 48px/280ms 交接跟随主导航排列', pageAnimation?.duration === 280 && (pageAnimation.frames as Keyframe[])[0].transform === `translate${pageAxis}(48px)`, { menuDirection: pageMenuDirection, animation: pageAnimation })
    const oldPageAnimation = animations.slice(start).find(item => item.target === home && (item.frames as Keyframe[])[1].transform)
    check('同层交接保持等距相反方向', oldPageAnimation?.duration === 280 && (oldPageAnimation.frames as Keyframe[])[1].transform === `translate${pageAxis}(-48px)`, oldPageAnimation)
    await settle()
    for (const path of ['/controller', '/xhome', '/setting', '/controller', '/setting', '/xhome', '/controller']) {
      requestPageFocus()
      await router.push(path)
      await wait(18)
    }
    await settle()
    state = snapshot()
    check('连续快切保留最新页面与唯一可操作焦点', state.route === 'controller' && state.activePages === 1 && !!state.focus && !node(state.focus!).closest('[inert]'), state)
    check('KeepAlive 恢复原组件，撤销临时定位和 inert', document.querySelector('.controller-page') === controller && !controller.hasAttribute('inert') && !(controller as HTMLElement).style.position, { sameInstance: document.querySelector('.controller-page') === controller, style: controller.getAttribute('style') })
    const panelStart = animations.length
    node('controller.tabs.settings').click()
    await settle()
    const menuDirection = getComputedStyle(document.querySelector('.controller-page .setting-sidebar__nav')!).flexDirection
    const panelAxis = menuDirection.startsWith('column') ? 'Y' : 'X'
    const panelAnimation = animations.slice(panelStart).find(item => item.target.parentElement?.classList.contains('controller-page__content') && (item.frames as Keyframe[])[0].transform)
    check('二级 24px/220ms 交接跟随分类菜单排列', panelAnimation?.duration === 220 && (panelAnimation.frames as Keyframe[])[0].transform === `translate${panelAxis}(24px)`, { menuDirection, animation: panelAnimation })
    const panelBackStart = animations.length
    node('controller.tabs.connection').click()
    await settle()
    check('二级返回方向与进入对称', animations.slice(panelBackStart).some(item => item.duration === 220 && (item.frames as Keyframe[])[0].transform === `translate${panelAxis}(-24px)`), { panelAxis, childCount: document.querySelector('.controller-page__content')!.children.length })
    await router.push('/setting')
    await settle()
    const settings = document.querySelector('.setting-page')
    check('缓存设置页恢复同一组件与滚动容器', settings === newPage && !newPage.hasAttribute('inert'), { sameInstance: settings === newPage })

    node('top-nav.profile').click()
    await frame()
    await frame()
    const panel = document.querySelector('.user-menu__panel')!
    const overlayAnimation = animations.filter(item => item.target === panel).at(-1)!
    check('弹层遮罩先入，面板 6px 升起，进入 180ms', overlayAnimation.duration === 160 && overlayAnimation.delay === 20 && (overlayAnimation.frames as Keyframe[])[0].transform === 'translateY(6px)', overlayAnimation)
    await settle()
    const scopeFade = animations.filter(item => item.target === ring()).at(-1)!
    check('跨弹层在新位置使用 100ms 淡入', scopeFade.duration === 100 && (scopeFade.frames as Keyframe[])[0].opacity === 0, { duration: scopeFade.duration, frames: scopeFade.frames })
    check('弹层独占焦点并暂停背景', navigationEngine.isTopmostScope('user.menu') && snapshot().ambient === 'paused', snapshot())
    node('user-menu.logout').click()
    await settle()
    check('确认弹层沿用共享过渡与独占 Scope', !!document.querySelector('.setting-modal-shell') && navigationEngine.getCurrentFocus()?.id?.endsWith('.cancel') === true, snapshot())
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    await frame()
    check('关闭确认即刻恢复来源，退出画面保持 inert', navigationEngine.getCurrentFocus()?.id === 'user-menu.logout' && document.querySelector('.setting-modal-shell')!.hasAttribute('inert'), snapshot())
    await settle(140)
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    await settle(150)
    check('返回源导航，弹层退出 120ms 后释放 DOM', navigationEngine.getCurrentFocus()?.id === 'top-nav.profile' && !document.querySelector('.user-menu-layer') && animations.some(item => item.duration === 120), snapshot())

    navigationEngine.focusElement(node('setting.content.general.inlineSelect.setting.items.general.theme.option.0'), false)
    await settle()
    const scroll = document.querySelector<HTMLElement>('.setting-panel__content')!
    scroll.scrollTop = 24
    await frame()
    await frame()
    state = snapshot()
    check('嵌套滚动即时吸附，外环保持对齐', state.maxDelta! < .6 && state.ringAnimations === 0, state)
    // 原生输入的隔离外层夹具，用同一实际 CSS、DOM 与引擎核对外环归属。
    const inputOutline = document.createElement('div')
    inputOutline.id = 'fixture.native-outline'
    inputOutline.className = 'ui-focus-outline'
    inputOutline.style.cssText = 'position:fixed;left:140px;top:60px;width:180px;height:48px;border-radius:6px;background:var(--ui-page-bg);z-index:10'
    const nativeInput = document.createElement('input')
    nativeInput.id = 'fixture.native-input'
    nativeInput.dataset.focusable = 'true'
    nativeInput.style.cssText = 'width:100%;height:100%'
    inputOutline.append(nativeInput)
    document.querySelector('.setting-page')!.append(inputOutline)
    navigationEngine.focusElement(nativeInput, false)
    await settle(150)
    state = snapshot()
    check('原生输入外环归属安全外层且保持唯一轮廓', state.focus === nativeInput.id && state.owner === inputOutline.id && state.owners === 1 && state.maxDelta! < .6 && getComputedStyle(inputOutline,'::before').opacity === '0', state)
    inputOutline.remove()
    const nativeRingAnimate = ring().animate
    Object.defineProperty(ring(), 'animate', { value: undefined, configurable: true })
    navigationEngine.focusElement(node('top-nav.setting'), false)
    await frame()
    check('WAAPI 能力缺失回到当前元素 CSS 外环', ring().hidden && document.querySelectorAll('[data-ui-focus-flow]').length === 0 && navigationEngine.getCurrentFocus()?.id === 'top-nav.setting', snapshot())
    Object.defineProperty(ring(), 'animate', { value: nativeRingAnimate, configurable: true })
    navigationEngine.focusElement(node('top-nav.xcloud'), false)
    await settle(120)

    inputController.setBackground(true)
    await frame()
    state = snapshot()
    check('窗口后台即使 keep-active 输入策略仍静止装饰', state.motion === 'static' && state.hidden && !state.transitions && state.ambient === 'paused', state)
    inputController.setBackground(false)
    await settle(120)
    check('窗口恢复后重新对齐当前焦点', snapshot().motion === 'active' && snapshot().maxDelta! < .6, snapshot())
    reducedPreference.matches = true
    reducedPreference.dispatchEvent(new Event('change'))
    await frame()
    check('减少动态效果保留逻辑焦点与静态 fallback', snapshot().motion === 'static' && ring().hidden && !!navigationEngine.getCurrentFocus(), snapshot())
    reducedPreference.matches = false
    reducedPreference.dispatchEvent(new Event('change'))
    await settle(120)
    await router.push('/xhome')
    await settle()
    requestPageFocus()
    await router.push('/xhome/fixture-console/games')
    await settle(400)
    const detail = document.querySelector('.library-page')!
    check('详情进入按深度向右推进', animations.some(item => item.target === detail && (item.frames as Keyframe[])[0].transform === 'translateX(48px)'), { route: router.currentRoute.value.name })
    await router.push('/xhome')
    await settle()
    check('详情返回自左进入并恢复首页缓存', document.querySelector('.home-page') === home && animations.some(item => item.target === home && (item.frames as Keyframe[])[0].transform === 'translateX(-48px)'), snapshot())
    await router.push('/xhome/stream/fixture-console')
    await frame()
    check('串流准备阶段进入静止状态', document.documentElement.dataset.uiMotion === 'static' && ring().hidden && !hasUiTransitions(), { motion: document.documentElement.dataset.uiMotion })
    await router.push('/my/album')
    await until(() => !!document.querySelector('[id^="album.clip."]'), '录像列表')
    await settle()
    const clip = document.querySelector<HTMLElement>('[id^="album.clip."]')!
    clip.click()
    await until(() => !!document.querySelector('video')?.readyState, '录像元数据')
    const video = document.querySelector('video')!
    video.muted = true
    node('album.preview.play').click()
    await until(() => !video.paused, '实际录像播放')
    await frame()
    check('录像播放事实暂停动效', snapshot().motion === 'static' && ring().hidden, { ...snapshot(), paused: video.paused })
    video.pause()
    await settle(120)
    check('录像暂停恢复焦点呈现', snapshot().motion === 'active' && snapshot().maxDelta! < .6, snapshot())
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    await settle(150)
    await router.push('/setting')
    await settle()
    node('setting.content.general.inlineSelect.setting.items.general.theme.option.1').click()
    await until(() => document.documentElement.dataset.theme === 'light', '浅色主题')
    await settle()
    state = snapshot()
    check('浅色主题的焦点与环境光保持同一合同', state.theme === 'light' && state.maxDelta! < .6 && state.owners === 1 && state.themeColorMatches, state)
    check('浏览器运行错误为零', errors.length === 0, errors)
  }
  catch (error) {
    output.textContent = JSON.stringify({ failed: String(error), records, errors }, null, 2)
  }
  finally {
    Element.prototype.animate = nativeAnimate
    finishUiTransitions()
    reducedPreference.matches = false
    reducedPreference.dispatchEvent(new Event('change'))
    inputController.setBackground(false)
    button.disabled = false
  }
}
document.getElementById('motion-hide')!.onclick = () => document.getElementById('motion-tools')!.hidden = true
