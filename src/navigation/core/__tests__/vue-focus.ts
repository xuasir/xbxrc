import type { VNode } from 'vue'
import type { FocusNode } from './focus-dom'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { ModuleKind, ScriptTarget, transpileModule } from 'typescript'
import { vi } from 'vitest'
import * as vue from 'vue'
import { createI18n } from 'vue-i18n'
import { compileScript, compileTemplate, parse } from 'vue/compiler-sfc'
import UiMotionTransition from '@/components/common/UiMotionTransition.vue'
import { navigationEngine } from '../engine'
import { focusFixture } from './focus-dom'

/** Vitest 加载真实 setup，客户端模板复用同一生产组件。 */
export function enableClientTemplate(component: object, filename: URL) {
  const { descriptor } = parse(readFileSync(filename, 'utf8'))
  const bindings = compileScript(descriptor, { id: 'vue-focus-test' }).bindings
  const template = compileTemplate({ source: descriptor.template!.content, filename: filename.pathname, id: 'vue-focus-test', compilerOptions: { bindingMetadata: bindings } })
  if (template.errors.length)
    throw new Error(template.errors.map(String).join('\n'))
  const code = template.code
    .replace(/import \{([^}]+)\} from "vue"/g, (_, names: string) => `const {${names.replace(/ as /g, ': ')}} = runtime`)
    .replace('export function render', 'function render')
  const Transition = vue.defineComponent({ inheritAttrs: false, setup: (_, { slots }) => () => slots.default?.() })
  const javascript = transpileModule(code, { compilerOptions: { module: ModuleKind.ESNext, target: ScriptTarget.ES2022 } }).outputText
  Object.assign(component, { render: runInNewContext(`${javascript}; render`, { runtime: { ...vue, Transition } }) })
}

export function vueFocusFixture() {
  enableClientTemplate(UiMotionTransition, new URL('../../../components/common/UiMotionTransition.vue', import.meta.url))
  const f = focusFixture()
  for (const key of ['focusElement', 'updateActiveScope', 'getCurrentFocus', 'getLastFocusFailure', 'cancelPendingRecovery', 'isTopmostScope', 'forgetFocus'] as const) {
    // 组件的共享单例使用同一受控 DOM 与引擎，保持真实 Scope 合同。
    vi.spyOn(navigationEngine, key).mockImplementation((...args: any[]) => (f.engine[key] as (...args: any[]) => any)(...args))
  }
  const isNode = (value: unknown): value is FocusNode => !!value && typeof value === 'object' && 'attrs' in value && value.attrs instanceof Map
  vi.stubGlobal('HTMLElement', class {
    static [Symbol.hasInstance](value: unknown) { return isNode(value) }
  })
  vi.stubGlobal('HTMLInputElement', class {
    static [Symbol.hasInstance](value: unknown) { return isNode(value) && value.tagName === 'input' }
  })
  type Listener = (event: Event) => void
  const children = new Map<FocusNode, FocusNode[]>()
  const listeners = new Map<FocusNode, Record<string, Listener | Listener[]>>()
  const windowListeners = new Map<string, Set<Listener>>()
  vi.mocked(window.addEventListener).mockImplementation((type, handler) => {
    if (!windowListeners.has(type))
      windowListeners.set(type, new Set())
    windowListeners.get(type)!.add(handler as Listener)
  })
  vi.mocked(window.removeEventListener).mockImplementation((type, handler) => windowListeners.get(type)?.delete(handler as Listener))
  vi.mocked(window.dispatchEvent).mockImplementation((event) => {
    let stopped = false
    event.stopImmediatePropagation = () => {
      stopped = true
    }
    for (const listener of windowListeners.get(event.type) ?? []) {
      listener(event)
      if (stopped)
        break
    }
    return !event.defaultPrevented
  })
  Object.assign(document, { addEventListener: vi.fn(), removeEventListener: vi.fn(), fullscreenEnabled: false })
  const fire = (element: FocusNode, type: string, values: Record<string, unknown> = {}) => {
    const event = new Event(type, { bubbles: true, cancelable: true })
    let stopped = false
    event.stopPropagation = () => {
      stopped = true
    }
    Object.defineProperty(event, 'target', { value: element })
    for (const [key, value] of Object.entries(values)) Object.defineProperty(event, key, { value, configurable: true })
    let current: FocusNode | null = element
    while (current) {
      Object.defineProperty(event, 'currentTarget', { value: current, configurable: true })
      const handler = listeners.get(current)?.[type]
      for (const listener of handler ? Array.isArray(handler) ? handler : [handler] : []) listener(event)
      current = !stopped && type !== 'focus' && type !== 'blur' ? current.parentElement : null
    }
    return event
  }
  const fireClick = (element: FocusNode, pointerType: string) => fire(element, 'click', { pointerType })
  const createNode = (tag: string) => {
    const element = vue.markRaw(f.node('', null, false, tag))
    Object.assign(element, { scrollTop: 0, scrollLeft: 0, scrollTo: vi.fn(), select: vi.fn() })
    children.set(element, [])
    listeners.set(element, {})
    const nativeFocus = element.focus.getMockImplementation() as () => void
    element.focus.mockImplementation(() => {
      const changed = f.active() !== element
      nativeFocus()
      if (changed)
        fire(element, 'focus')
    })
    element.dispatchEvent.mockImplementation(event => !fire(element, event.type).defaultPrevented)
    element.click = vi.fn(() => fireClick(element, ''))
    if (tag === 'video') {
      Object.assign(element, {
        currentTime: 0,
        duration: Number.NaN,
        volume: 1,
        muted: false,
        paused: true,
        ended: false,
        play: vi.fn(async () => {
          Object.assign(element, { paused: false })
          fire(element, 'play')
        }),
        pause: vi.fn(() => {
          Object.assign(element, { paused: true })
          fire(element, 'pause')
        }),
      })
    }
    return element
  }
  const detach = (element: FocusNode) => {
    const siblings = element.parentElement ? children.get(element.parentElement) : undefined
    if (siblings) {
      const index = siblings.indexOf(element)
      if (index >= 0)
        siblings.splice(index, 1)
    }
  }
  const body = createNode('body')
  const renderer = vue.createRenderer<FocusNode, FocusNode>({
    createElement: createNode,
    createText: () => createNode('#text'),
    createComment: () => createNode('#comment'),
    setText: (element, value) => element.attrs.set('text', value),
    setElementText: (element, value) => element.attrs.set('text', value),
    setScopeId: (element, id) => element.setAttribute(id, ''),
    parentNode: element => element.parentElement,
    nextSibling: element => element.parentElement ? children.get(element.parentElement)?.[children.get(element.parentElement)!.indexOf(element) + 1] ?? null : null,
    querySelector: selector => selector === 'body' ? body : document.querySelector(selector) as unknown as FocusNode,
    patchProp: (element, key, _, value) => {
      if (/^on[A-Z]/.test(key)) {
        const type = key.slice(2).replace(/^[A-Z]/, char => char.toLowerCase()).replace(/[A-Z]/g, char => `-${char.toLowerCase()}`)
        listeners.get(element)![type] = value
        return
      }
      if (key === 'id')
        element.id = value ?? ''
      if (key === 'disabled')
        element.disabled = value === true
      if (value === null || value === undefined || value === false)
        element.removeAttribute(key)
      else element.setAttribute(key, String(value))
    },
    insert: (element, parent, anchor) => {
      detach(element)
      const siblings = children.get(parent)!
      const index = anchor ? siblings.indexOf(anchor) : -1
      siblings.splice(index < 0 ? siblings.length : index, 0, element)
      element.parentElement = parent
      element.connected = true
    },
    remove: (element) => {
      detach(element)
      element.parentElement = null
      element.connected = false
    },
  })
  const root = createNode('main')
  const app = renderer.createApp({ render: () => null })
  app.provide(vue.ssrContextKey, { modules: new Set() })
  app.use(createI18n({ legacy: false, locale: 'zh', messages: { zh: {} }, missingWarn: false, fallbackWarn: false }))
  const render = (vnode: VNode | null) => {
    if (vnode)
      vnode.appContext = app._context
    renderer.render(vnode, root)
  }
  f.engine.start()
  return { ...f, render, fire, fireClick, dispose: () => {
    render(null)
    f.engine.stop()
  }, find: (id: string) => {
    const element = document.getElementById(id)
    if (!element || !document.contains(element))
      throw new Error(`导航测试节点缺失：${id}`)
    return element as unknown as FocusNode
  } }
}
