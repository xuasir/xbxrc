import type { VNode } from 'vue'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { afterEach, describe, expect, it, vi } from 'vitest'
import * as vue from 'vue'
import { compileTemplate, parse } from 'vue/compiler-sfc'
import StreamActionSheet from '@/components/stream/StreamActionSheet.vue'
import { focusFixture } from '@/navigation/core/__tests__/focus-dom'
import { enableClientTemplate, vueFocusFixture } from '@/navigation/core/__tests__/vue-focus'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { inputUiTrace } from '@/navigation/core/input-trace'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '@/navigation/spatial-nav.constants'

vi.mock('@/navigation/core/haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
vi.mock('vue', async (original) => {
  const actual = await original<typeof import('vue')>()
  return {
    ...actual,
    Transition: actual.defineComponent({ inheritAttrs: false, setup: (_, { slots }) => () => slots.default?.() }),
  }
})

// 编译生产页面的父 Scope 配置，覆盖模板与几何引擎之间的可交互性合同。
const { descriptor } = parse(readFileSync(new URL('../Stream.vue', import.meta.url), 'utf8'))
const root = descriptor.template!.ast!.children.find(node => node.type === 1)
if (!root || root.type !== 1 || root.tag !== 'FocusScope')
  throw new Error('串流页需要提供父 FocusScope')
const compiled = compileTemplate({
  source: `<FocusScope ${root.props.map(prop => prop.loc.source).join(' ')} />`,
  filename: 'StreamScope.vue',
  id: 'stream-scope-regression',
})
const code = compiled.code
  .replace(/import \{([^}]+)\} from "vue"/g, (_, names: string) => `const {${names.replace(/ as /g, ': ')}} = runtime`)
  .replace('export function render', 'function render')
const renderScope = runInNewContext(`${code}; render`, {
  runtime: { ...vue, resolveComponent: () => 'section' },
}) as (context: object, cache: unknown[]) => VNode

enableClientTemplate(FocusScope, new URL('../../navigation/core/vue/FocusScope.vue', import.meta.url))
enableClientTemplate(Focusable, new URL('../../navigation/core/vue/Focusable.vue', import.meta.url))
enableClientTemplate(StreamActionSheet, new URL('../../components/stream/StreamActionSheet.vue', import.meta.url))

const engines: ReturnType<typeof focusFixture>['engine'][] = []
const unmounts: Array<() => void> = []
afterEach(() => {
  for (const unmount of unmounts.splice(0)) unmount()
  for (const engine of engines.splice(0)) engine.stop()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

function pointerFixture() {
  const f = vueFocusFixture()
  unmounts.push(f.dispose)
  return f
}

describe('串流弹层与父焦点域', () => {
  it.each(['stream.action-sheet', 'stream.diagnostics-menu-sheet'])(
    '%s 旧父域的触屏失败提供 inactiveAncestorScope，修复后记录触屏成功',
    async (sheetId) => {
      const f = pointerFixture()
      const rows: Array<Record<string, unknown>> = []
      const detach = inputUiTrace.attach({
        enabled: () => true,
        write: (_event, payload) => {
          rows.push(payload)
          return true
        },
        input: () => ({
          owner: 'ui',
          backend: 'browser',
          activeDeviceId: 'fixture',
          exclusiveCapture: false,
        }),
        anomaly: () => {
        },
      })
      const selected = vi.fn(() => inputUiTrace.result('operationCompleted'))
      const render = (active: boolean) => f.render(vue.h(FocusScope, { id: 'stream.page', active }, { default: () => [
        vue.h(StreamActionSheet, { open: true, scopeId: sheetId, title: '菜单', items: [{ id: 'display', label: '显示设置' }], onSelect: selected }),
      ] }))
      try {
        render(false)
        await vue.nextTick()
        await vue.nextTick()
        f.fireClick(f.find('display'), 'touch')
        expect(selected).not.toHaveBeenCalled()
        expect(rows.some(row => row.source === 'touch' && row.reason === 'inactiveAncestorScope')).toBe(true)
        render(true)
        await vue.nextTick()
        await vue.nextTick()
        f.fireClick(f.find('display'), 'touch')
        expect(selected).toHaveBeenCalledOnce()
        expect(rows.some(row => row.source === 'touch' && row.outcome === 'operationCompleted' && row.action === 'display')).toBe(true)
      }
      finally { detach() }
    },
  )

  it.each(['stream.action-sheet', 'stream.diagnostics-menu-sheet'])(
    '%s 的真实菜单组件接收触屏点击、鼠标点击和手柄确认',
    async (sheetId) => {
      const f = pointerFixture()
      const selected = vi.fn()
      const renderView = (open: boolean) => {
        const props = renderScope({ SPATIAL_NAV_SCOPE_IDS, defaultFocusId: 'entry', hasInteractiveSheet: open }, []).props!
        f.render(vue.h(FocusScope, props, { default: () => [
          vue.h(Focusable, { id: 'entry', as: 'button' }),
          vue.h(StreamActionSheet, { open, scopeId: sheetId, title: '测试菜单', items: [{ id: 'first', label: '操作一' }, { id: 'second', label: '操作二' }], onSelect: selected }),
        ] }))
      }
      renderView(false)
      await vue.nextTick()
      await vue.nextTick()
      renderView(true)
      await vue.nextTick()
      await vue.nextTick()

      const touch = f.fireClick(f.find('first'), 'touch')
      expect(touch.defaultPrevented).toBe(false)
      expect(selected.mock.calls).toEqual([['first']])
      const mouse = f.fireClick(f.find('second'), 'mouse')
      expect(mouse.defaultPrevented).toBe(false)
      expect(selected.mock.calls).toEqual([['first'], ['second']])
      f.find('second').getBoundingClientRect = () => ({ left: 300, right: 400, top: 310, bottom: 350, width: 100, height: 40 })
      f.focus(f.find('first'))
      inputDispatcher.dispatch(NavigationIntent.Down)
      inputDispatcher.dispatch(NavigationIntent.Action)
      expect(selected.mock.calls).toEqual([['first'], ['second'], ['second']])

      const blockedBackground = f.fireClick(f.find('entry'), 'touch')
      expect(blockedBackground.defaultPrevented).toBe(true)
      expect(selected).toHaveBeenCalledTimes(3)
    },
  )

  it.each(['stream.action-sheet', 'stream.diagnostics-menu-sheet'])(
    '%s 打开后接受点击、方向及确认，关闭后恢复串流承接节点',
    (sheetId) => {
      const f = focusFixture()
      engines.push(f.engine)
      f.engine.start()
      const page = f.zone(SPATIAL_NAV_SCOPE_IDS.streamPage)
      const entry = f.node(SPATIAL_NAV_NODE_IDS.streamPage.menu, page)
      const sink = f.node(SPATIAL_NAV_NODE_IDS.streamPage.focusSink, page, false)
      sink.attrs.set('data-nav-focus-sink', 'true')
      sink.attrs.set('aria-hidden', 'true')
      const applyPage = (hasInteractiveSheet: boolean) => {
        const props = renderScope({ SPATIAL_NAV_SCOPE_IDS, defaultFocusId: entry.id, hasInteractiveSheet }, []).props!
        const active = props.active ?? true
        page.attrs.set('data-nav-zone-active', String(active))
        f.engine.updateActiveScope(page.id, active)
      }
      applyPage(false)
      f.focus(entry)

      const sheet = f.zone(sheetId, page)
      const first = f.node('first-action', sheet)
      const second = f.node('second-action', sheet)
      second.getBoundingClientRect = () => ({ left: 300, right: 400, top: 310, bottom: 350, width: 100, height: 40 })
      sheet.attrs.set('data-nav-zone-active', 'true')
      applyPage(true)
      f.engine.updateActiveScope(sheetId, true)

      expect(f.engine.isTopmostScope(sheetId)).toBe(true)
      expect(f.focus(first)).toBe(true)
      expect(f.focus(entry)).toBe(false)
      inputDispatcher.dispatch(NavigationIntent.Down)
      expect(f.engine.getCurrentFocus()).toBe(second)
      inputDispatcher.dispatch(NavigationIntent.Action)
      expect(second.click).toHaveBeenCalledOnce()

      sheet.connected = false
      f.engine.updateActiveScope(sheetId, false)
      applyPage(false)
      expect(f.focus(sink)).toBe(true)
      expect(f.engine.isTopmostScope(page.id)).toBe(true)
      expect(first.attrs.has('data-focused')).toBe(false)
      expect(second.attrs.has('data-focused')).toBe(false)
    },
  )
})
