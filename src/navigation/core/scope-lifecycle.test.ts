import { afterEach, describe, expect, it, vi } from 'vitest'
import { createRenderer, h, KeepAlive, nextTick, ssrContextKey } from 'vue'
import { navigationEngine } from './engine'
import FocusScope from './vue/FocusScope.vue'

vi.mock('./haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))

interface HostNode { children: HostNode[], parent: HostNode | null, text: string, setAttribute: ReturnType<typeof vi.fn> }
const node = (text = ''): HostNode => ({ children: [], parent: null, text, setAttribute: vi.fn() })
function fixture() {
  const update = vi.spyOn(navigationEngine, 'updateActiveScope').mockImplementation(() => {})
  const renderer = createRenderer<HostNode, HostNode>({
    createElement: tag => node(tag),
    createText: text => node(text),
    createComment: text => node(text),
    setText: (el, text) => { el.text = text },
    setElementText: (el, text) => { el.text = text },
    parentNode: el => el.parent,
    nextSibling: el => el.parent?.children[el.parent.children.indexOf(el) + 1] ?? null,
    patchProp: () => {},
    insert: (el, parent, anchor) => {
      if (el.parent)
        el.parent.children.splice(el.parent.children.indexOf(el), 1)
      const index = anchor ? parent.children.indexOf(anchor) : -1
      parent.children.splice(index < 0 ? parent.children.length : index, 0, el)
      el.parent = parent
    },
    remove: (el) => {
      if (el.parent)
        el.parent.children.splice(el.parent.children.indexOf(el), 1)
      el.parent = null
    },
  })
  const root = node()
  const app = renderer.createApp({ render: () => null })
  app.provide(ssrContextKey, { modules: new Set() })
  return { update, render: (vnode: Parameters<typeof renderer.render>[0]) => {
    if (vnode)
      vnode.appContext = app._context
    renderer.render(vnode, root)
  } }
}
afterEach(() => vi.restoreAllMocks())

describe('focusScope 生命周期不变量（真实 Vue hooks）', () => {
  it('对照：普通挂载和卸载依次激活、释放焦点域', async () => {
    const f = fixture()
    f.render(h(FocusScope, { id: 'normal', active: true }))
    await nextTick()
    await nextTick()
    expect(f.update).toHaveBeenCalledWith('normal', true, expect.objectContaining({ owner: expect.any(Symbol) }))
    f.render(null)
    await nextTick()
    expect(f.update.mock.calls.at(-1)).toEqual(['normal', false, expect.objectContaining({ owner: expect.any(Symbol) })])
  })
  it('卸载后，挂载时等待的 nextTick 释放旧激活请求', async () => {
    const f = fixture()
    f.render(h(FocusScope, { id: 'race-unmount', active: true }))
    f.render(null)
    await nextTick()
    await nextTick()
    expect(f.update.mock.calls.at(-1)).toEqual(['race-unmount', false, expect.objectContaining({ owner: expect.any(Symbol) })])
  })
  it('active 关闭后，挂载时等待的 nextTick 保持关闭', async () => {
    const f = fixture()
    f.render(h(FocusScope, { id: 'race-active', active: true }))
    f.render(h(FocusScope, { id: 'race-active', active: false }))
    await nextTick()
    await nextTick()
    expect(f.update.mock.calls.at(-1)).toEqual(['race-active', false, expect.objectContaining({ owner: expect.any(Symbol) })])
  })
  it('keepAlive 停用时释放焦点域，恢复时重新激活', async () => {
    const f = fixture()
    const view = (active: boolean) => h(KeepAlive, null, { default: () => active ? h(FocusScope, { id: 'cached-scope', active: true }) : null })
    f.render(view(true))
    await nextTick()
    await nextTick()
    f.update.mockClear()
    f.render(view(false))
    await nextTick()
    expect(f.update).toHaveBeenCalledWith('cached-scope', false, expect.objectContaining({ owner: expect.any(Symbol) }))
    f.render(view(true))
    await nextTick()
    expect(f.update).toHaveBeenCalledWith('cached-scope', true, expect.objectContaining({ owner: expect.any(Symbol) }))
  })
})
