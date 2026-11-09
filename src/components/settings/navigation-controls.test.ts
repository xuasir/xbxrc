import type { RenderControlsValue } from '@shared/config/render-controls'
import type { FocusNode } from '@/navigation/core/__tests__/focus-dom'
import type { SettingIndexedSection } from '@/pages/settings/setting-types'
import type { StreamRenderState } from '@/streaming/runtime/runtime-contract'
import type { StreamExperienceMetricsViewModel, StreamSessionDiagnosticsSnapshot } from '@/streaming/types'
import { normalizeRenderControls } from '@shared/config/render-controls'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, KeepAlive, nextTick, ref } from 'vue'
import StreamActionSheet from '@/components/stream/StreamActionSheet.vue'
import StreamAudioSheet from '@/components/stream/StreamAudioSheet.vue'
import StreamBrowserDiagnosticsPanel from '@/components/stream/StreamBrowserDiagnosticsPanel.vue'
import StreamExperiencePanel from '@/components/stream/StreamExperiencePanel.vue'
import StreamTextSheet from '@/components/stream/StreamTextSheet.vue'
import { enableClientTemplate, vueFocusFixture } from '@/navigation/core/__tests__/vue-focus'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import LibraryRefresh from '@/pages/library/LibraryRefresh.vue'
import LibrarySelectControl from '@/pages/library/LibrarySelectControl.vue'
import SettingSectionList from '@/pages/settings/SettingSectionList.vue'
import { buildStreamBrowserDiagnosticsViewModel } from '@/streaming/stream-panel-view-models'
import SettingDisplayOptionsSheet from './SettingDisplayOptionsSheet.vue'
import SettingInlineSingleSelect from './SettingInlineSingleSelect.vue'
import SettingInputControl from './SettingInputControl.vue'
import SettingKeyboardSheet from './SettingKeyboardSheet.vue'
import SettingModalShell from './SettingModalShell.vue'
import SettingRow from './SettingRow.vue'
import SettingSingleSelectPopupSheet from './SettingSingleSelectPopupSheet.vue'
import SettingToggleRow from './SettingToggleRow.vue'

const input = vi.hoisted(() => ({ source: 'gamepad' }))
vi.mock('@/navigation/core/haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
vi.mock('@shared/gamepad/runtime', () => ({ inputController: {
  getSnapshot: () => ({ preferences: { controlSource: input.source } }),
  subscribe: () => () => {},
} }))
const components = [
  [Focusable, '../../navigation/core/vue/Focusable.vue'],
  [FocusScope, '../../navigation/core/vue/FocusScope.vue'],
  [SettingModalShell, './SettingModalShell.vue'],
  [SettingKeyboardSheet, './SettingKeyboardSheet.vue'],
  [SettingInputControl, './SettingInputControl.vue'],
  [SettingSingleSelectPopupSheet, './SettingSingleSelectPopupSheet.vue'],
  [SettingRow, './SettingRow.vue'],
  [SettingToggleRow, './SettingToggleRow.vue'],
  [SettingInlineSingleSelect, './SettingInlineSingleSelect.vue'],
  [SettingSectionList, '../../pages/settings/SettingSectionList.vue'],
  [LibraryRefresh, '../../pages/library/LibraryRefresh.vue'],
  [LibrarySelectControl, '../../pages/library/LibrarySelectControl.vue'],
  [StreamActionSheet, '../stream/StreamActionSheet.vue'],
  [StreamTextSheet, '../stream/StreamTextSheet.vue'],
  [StreamAudioSheet, '../stream/StreamAudioSheet.vue'],
  [SettingDisplayOptionsSheet, './SettingDisplayOptionsSheet.vue'],
  [StreamExperiencePanel, '../stream/StreamExperiencePanel.vue'],
  [StreamBrowserDiagnosticsPanel, '../stream/StreamBrowserDiagnosticsPanel.vue'],
] as const
for (const [component, path] of components) enableClientTemplate(component, new URL(path, import.meta.url))
const fixtures: ReturnType<typeof vueFocusFixture>[] = []
function fixture() {
  const f = vueFocusFixture()
  fixtures.push(f)
  return f
}
async function settle() {
  for (let i = 0; i < 3; i++) await nextTick()
}
afterEach(() => {
  for (const f of fixtures.splice(0)) f.dispose()
  input.source = 'gamepad'
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('渲染控制与可开关顶部指标栏', () => {
  it('触屏、鼠标与手柄通过同一三档控件互斥调整 SR 和锐化，颜色独立', async () => {
    const f = fixture()
    const change = vi.fn()
    const close = vi.fn()
    f.render(h(SettingDisplayOptionsSheet, { open: true, scopeId: 'render', title: '画面', currentValue: normalizeRenderControls({}), onChange: change, onClose: close }))
    // 真实 DOM 的观察器会在异步 Scope 注册前看到弹层节点。
    f.mutation()
    await settle()
    expect(f.engine.getCurrentFocus()?.id).toBe('render.sr')
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(change.mock.calls.at(-1)?.[0].mode).toBe('sr')
    f.fireClick(f.find('render.sharpening.clear'), 'touch')
    await settle()
    expect(change.mock.calls.at(-1)?.[0]).toMatchObject({ mode: 'sharpen', displayOptions: { sharpness: 3 } })
    f.find('render.sharpening.soft').getBoundingClientRect = () => ({ left: 450, right: 550, top: 250, bottom: 290, width: 100, height: 40 })
    f.focus(f.find('render.sharpening.clear'))
    inputDispatcher.dispatch(NavigationIntent.Right)
    expect(f.engine.getCurrentFocus()?.id).toBe('render.sharpening.soft')
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(change.mock.calls.at(-1)?.[0]).toMatchObject({ mode: 'sharpen', displayOptions: { sharpness: 1 } })
    f.fireClick(f.find('render.preset.clear'), 'mouse')
    await settle()
    expect(change.mock.calls.at(-1)?.[0]).toMatchObject({ mode: 'sharpen', displayOptions: { sharpness: 1, saturation: 105 } })
    f.fireClick(f.find('render.video'), 'touch')
    await settle()
    expect(change.mock.calls.at(-1)?.[0].mode).toBe('video')
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(close).toHaveBeenCalledOnce()
  })

  it('本次应用与显式默认保存分开提交，busy 期间保持焦点并吸收重复操作', async () => {
    const f = fixture()
    const loading = ref(false)
    const submit = vi.fn((_value: RenderControlsValue, _saveAsDefault: boolean) => loading.value = true)
    const restore = vi.fn()
    f.render(h(defineComponent(() => () => h(SettingDisplayOptionsSheet, { open: true, scopeId: 'render', title: '画面', currentValue: normalizeRenderControls({ render_mode: 'sr' }), configurationScope: 'session', loading: loading.value, onSubmit: submit, onRestoreDefaults: restore }))))
    await settle()
    f.focus(f.find('render.submit'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(submit.mock.calls[0]?.[1]).toBe(false)
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(submit).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('render.submit')
    loading.value = false
    await settle()
    f.fireClick(f.find('render.save-default'), 'touch')
    await settle()
    expect(submit.mock.calls[1]?.[1]).toBe(true)
    loading.value = false
    await settle()
    f.fireClick(f.find('render.restore'), 'mouse')
    expect(restore).toHaveBeenCalledOnce()
  })

  it('实际模式与回退原因随顶部指标栏开关显示，SR 候选等待时保持已提交模式', async () => {
    const f = fixture()
    const visible = ref(true)
    const state = ref<StreamRenderState>({ mode: 'video', pending: true })
    f.render(h(defineComponent(() => () => h(StreamExperiencePanel, { visible: visible.value, mount: { phase: 'mounted' } as any, model: { notice: undefined } as StreamExperienceMetricsViewModel, renderState: state.value }))))
    await settle()
    const renderCells = () => Array.from(document.querySelectorAll('span')).filter(node => document.contains(node) && (node as unknown as FocusNode).attrs.get('class')?.includes('stream-runtime-bar__render')) as unknown as FocusNode[]
    expect(renderCells()[0]?.attrs.get('text')).toContain('setting.renderControls.switching')
    state.value = { mode: 'sr', pending: false }
    await settle()
    expect(renderCells()[0]?.attrs.get('text')).toBe('SR')
    state.value = { mode: 'video', pending: false, fallbackReason: 'performance' }
    await settle()
    expect(renderCells()[0]?.attrs.get('text')).toContain('setting.renderControls.fallback.performance')
    visible.value = false
    await settle()
    expect(renderCells()).toHaveLength(0)
  })
})

describe('异步操作保持来源焦点', () => {
  it('真实设置开关保存中保持唯一高亮，重复确认合并，用户移动后保持新位置', async () => {
    const f = fixture()
    const pending = ref<string | null>(null)
    const confirm = vi.fn(() => pending.value = 'toggle')
    const sections = [{ key: 'general', label: '通用', entries: [{ kind: 'field', row: { key: 'toggle', nodeId: 'toggle', label: '开关', control: 'toggle', value: true } }] }] as SettingIndexedSection[]
    f.render(h(defineComponent(() => () => h(FocusScope, { id: 'page' }, () => [
      h(SettingSectionList, { sections, scopeId: 'page', pendingActionKey: pending.value, expanded: new Set<string>(), onRowConfirm: confirm }),
      h(Focusable, { id: 'next', as: 'button' }, () => '其他操作'),
    ]))))
    await settle()
    f.focus(f.find('toggle'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    f.fireClick(f.find('toggle'), 'touch')
    expect(confirm).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('toggle')
    expect(f.find('toggle').disabled).toBe(false)
    expect(f.find('toggle').attrs.get('aria-busy')).toBe('true')
    f.find('next').getBoundingClientRect = () => ({ left: 300, right: 400, top: 310, bottom: 350, width: 100, height: 40 })
    inputDispatcher.dispatch(NavigationIntent.Down)
    pending.value = null
    await settle()
    f.mutation()
    expect(f.engine.getCurrentFocus()?.id).toBe('next')
  })

  it('真实刷新入口合并 A 和鼠标重复请求，结束后继续接收确认', async () => {
    const f = fixture()
    const busy = ref(false)
    const refresh = vi.fn(() => busy.value = true)
    f.render(h(defineComponent(() => () => h(LibraryRefresh, { id: 'refresh', busy: busy.value, onRefresh: refresh }))))
    f.focus(f.find('refresh'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    f.fireClick(f.find('refresh'), 'mouse')
    expect(refresh).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('refresh')
    busy.value = false
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(refresh).toHaveBeenCalledTimes(2)
  })

  it('真实菜单请求期间继续持有焦点，能力限制继续退出导航候选', async () => {
    const f = fixture()
    const busy = ref(false)
    const select = vi.fn(() => busy.value = true)
    f.render(h(defineComponent(() => () => h(StreamActionSheet, { open: true, scopeId: 'menu', title: '菜单', items: [{ id: 'action', label: '操作', busy: busy.value }, { id: 'unavailable', label: '能力限制', disabled: true }], onSelect: select }))))
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(select).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('action')
    expect(f.engine.getFocusableElements(document).map(element => element.id)).toEqual(['action'])
  })

  it.each(['fulfilled', 'rejected'])('诊断复制 %s 的迟到结果保持新弹层请求归属，B 返回原入口', async (outcome) => {
    const f = fixture()
    const visible = ref(false)
    const requests: Array<{ resolve: () => void, reject: (error: Error) => void }> = []
    const writeText = vi.fn(() => new Promise<void>((resolve, reject) => requests.push({ resolve, reject })))
    vi.stubGlobal('navigator', { clipboard: { writeText } })
    const model = buildStreamBrowserDiagnosticsViewModel({
      snapshot: null,
      diagnostics: {} as StreamSessionDiagnosticsSnapshot,
      i18n: { t: key => key, te: () => false },
    })
    f.render(h(defineComponent(() => () => h(FocusScope, { id: 'page' }, () => [
      h(Focusable, { id: 'entry', as: 'button' }, () => '诊断'),
      h(StreamBrowserDiagnosticsPanel, { visible: visible.value, mount: { phase: 'mounted' }, model, group: 'network', onClose: () => visible.value = false }),
    ]))))
    await settle()
    f.focus(f.find('entry'))
    visible.value = true
    await settle()
    f.focus(f.find('stream.diagnostic-details.copy'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Action)
    f.fireClick(f.find('stream.diagnostic-details.copy'), 'mouse')
    expect(writeText).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('stream.diagnostic-details.copy')
    expect(f.find('stream.diagnostic-details.copy').attrs.get('data-nav-busy')).toBe('true')
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(visible.value).toBe(false)
    expect(f.engine.getCurrentFocus()?.id).toBe('entry')
    visible.value = true
    await settle()
    f.focus(f.find('stream.diagnostic-details.copy'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    if (outcome === 'fulfilled')
      requests[0].resolve()
    else requests[0].reject(new Error('旧复制失败'))
    await settle()
    expect(writeText).toHaveBeenCalledTimes(2)
    expect(f.find('stream.diagnostic-details.copy').attrs.get('data-nav-busy')).toBe('true')
    requests[1].resolve()
    await settle()
    expect(f.find('stream.diagnostic-details.copy').attrs.has('data-nav-busy')).toBe(false)
    expect(f.engine.getCurrentFocus()?.id).toBe('stream.diagnostic-details.copy')
  })

  it.each(['pointerdown', 'click'])('%s 同一回退节点取消旧来源恢复', async (type) => {
    const f = fixture()
    const disabled = ref(false)
    f.render(h(defineComponent(() => () => h('div', [h(Focusable, { id: 'source', as: 'button', disabled: disabled.value }), h(Focusable, { id: 'fallback', as: 'button' })]))))
    f.focus(f.find('source'))
    disabled.value = true
    await settle()
    f.mutation()
    f.fire(f.find('fallback'), type, { pointerType: 'mouse' })
    disabled.value = false
    await settle()
    f.mutation()
    expect(f.engine.getCurrentFocus()?.id).toBe('fallback')
  })
})

describe('单选弹层与来源恢复', () => {
  it('a 打开当前选项，方向选择与 A 回填，B 取消并恢复稳定入口', async () => {
    const f = fixture()
    const value = ref('one')
    const update = vi.fn((next: string) => value.value = next)
    const options = [{ value: 'one', label: '选项一' }, { value: 'two', label: '选项二' }]
    f.render(h(defineComponent(() => () => h(LibrarySelectControl, { 'id': 'filter', 'label': '筛选', 'modelValue': value.value, options, 'onUpdate:modelValue': update }))))
    f.focus(f.find('filter'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(f.engine.getCurrentFocus()?.id).toBe('filter.options.option.0')
    f.find('filter.options.option.1').getBoundingClientRect = () => ({ left: 300, right: 400, top: 310, bottom: 350, width: 100, height: 40 })
    inputDispatcher.dispatch(NavigationIntent.Down)
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(value.value).toBe('two')
    expect(update).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('filter')
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(value.value).toBe('two')
    expect(update).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('filter')
  })

  it('选项动态移除后只提交当前有效选项，空列表关闭弹层', async () => {
    const f = fixture()
    const options = ref([{ value: 'one', label: '一' }, { value: 'two', label: '二' }])
    const update = vi.fn()
    f.render(h(defineComponent(() => () => h(LibrarySelectControl, { 'id': 'filter', 'label': '筛选', 'modelValue': 'one', 'options': options.value, 'onUpdate:modelValue': update }))))
    f.focus(f.find('filter'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    f.focus(f.find('filter.options.option.1'))
    options.value = [{ value: 'one', label: '一' }]
    await settle()
    f.mutation()
    expect(f.engine.getCurrentFocus()?.id).toBe('filter.options.option.0')
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(update).toHaveBeenCalledWith('one')
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    options.value = []
    await settle()
    f.mutation()
    expect(f.engine.isTopmostScope('filter.options')).toBe(false)
    expect(update).toHaveBeenCalledOnce()
  })

  it('keepAlive 停用释放弹层，重新激活使用关闭状态', async () => {
    const f = fixture()
    const active = ref(true)
    f.render(h(defineComponent(() => () => h(KeepAlive, null, () => active.value ? h(LibrarySelectControl, { id: 'filter', label: '筛选', modelValue: 'one', options: [{ value: 'one', label: '一' }] }) : null))))
    f.focus(f.find('filter'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    active.value = false
    await settle()
    expect(f.engine.isTopmostScope('filter.options')).toBe(false)
    active.value = true
    await settle()
    expect(f.find('filter').attrs.get('aria-expanded')).toBeUndefined()
  })
})

describe('共享键盘与父草稿', () => {
  it('搜索 A 打开键盘，完成只回填父草稿，B 取消键盘草稿', async () => {
    const f = fixture()
    const value = ref('搜索')
    const submit = vi.fn()
    f.render(h(defineComponent(() => () => h(SettingInputControl, { 'id': 'search', 'scopeId': 'page', 'label': '搜索', 'type': 'search', 'modelValue': value.value, 'onUpdate:modelValue': (next: string) => value.value = next, 'onSubmit': submit }))))
    f.focus(f.find('search'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(f.engine.getCurrentFocus()?.id).toBe('search.keyboard.key.0.0')
    inputDispatcher.dispatch(NavigationIntent.Action)
    f.fireClick(f.find('search.keyboard.done'), 'mouse')
    await settle()
    expect(value.value).toBe('搜索1')
    expect(submit).not.toHaveBeenCalled()
    expect(f.engine.getCurrentFocus()?.id).toBe('search')
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    f.fireClick(f.find('search.keyboard.clear'), 'mouse')
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(value.value).toBe('搜索1')
    expect(f.engine.getCurrentFocus()?.id).toBe('search')
  })

  it('键盘模拟使用 readonly 导航，鼠标直接编辑与 B 退出保留输入', async () => {
    input.source = 'keyboard'
    const f = fixture()
    const update = vi.fn()
    const submit = vi.fn()
    f.render(h(SettingInputControl, { 'id': 'field', 'scopeId': 'page', 'label': '字段', 'modelValue': '初值', 'onUpdate:modelValue': update, 'onSubmit': submit }))
    expect(f.find('field').attrs.get('readonly')).toBe('true')
    f.fire(f.find('field'), 'pointerdown', { pointerType: 'mouse' })
    await settle()
    expect(f.find('field').attrs.has('readonly')).toBe(false)
    Object.assign(f.find('field'), { value: '输入' })
    f.fire(f.find('field'), 'input')
    f.fire(f.find('field'), 'keydown', { key: 'Enter' })
    expect(update).toHaveBeenCalledWith('输入')
    expect(submit).toHaveBeenCalledOnce()
    f.focus(f.find('field'))
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(f.find('field').attrs.get('readonly')).toBe('true')
    expect(f.engine.getCurrentFocus()?.id).toBe('field')
  })

  it.each(['busy', 'disabled', 'active'] as const)('%s 生命周期变化关闭旧键盘并阻止旧草稿提交', async (key) => {
    const f = fixture()
    const blocked = ref(false)
    const update = vi.fn()
    f.render(h(defineComponent(() => () => h(SettingInputControl, { 'id': 'field', 'scopeId': 'page', 'label': '字段', 'modelValue': '', 'busy': key === 'busy' && blocked.value, 'disabled': key === 'disabled' && blocked.value, 'active': key !== 'active' || !blocked.value, 'onUpdate:modelValue': update }))))
    f.focus(f.find('field'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    blocked.value = true
    await settle()
    expect(f.engine.isTopmostScope('field.keyboard')).toBe(false)
    f.fire(f.find('field'), 'keydown', { key: 'Enter' })
    expect(update).not.toHaveBeenCalled()
  })

  it('串流文字键盘先回填草稿，发送期间重复 A 合并，B 保留父文字弹层', async () => {
    const f = fixture()
    const loading = ref(false)
    const submit = vi.fn(() => loading.value = true)
    const close = vi.fn()
    f.render(h(defineComponent(() => () => h(StreamTextSheet, { open: true, scopeId: 'text', loading: loading.value, onSubmit: submit, onClose: close }))))
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Action)
    f.fireClick(f.find('text.field.keyboard.done'), 'mouse')
    await settle()
    expect(submit).not.toHaveBeenCalled()
    expect(f.engine.getCurrentFocus()?.id).toBe('text.field')
    f.fireClick(f.find('text.submit'), 'mouse')
    await settle()
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(submit.mock.calls).toEqual([['1']])
    expect(f.engine.getCurrentFocus()?.id).toBe('text.submit')
    loading.value = false
    await settle()
    f.focus(f.find('text.field'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(close).not.toHaveBeenCalled()
    expect(f.engine.getCurrentFocus()?.id).toBe('text.field')
  })

  it('串流音量数字键盘替换数值，父层限制 0–10，取消保持原值', async () => {
    const f = fixture()
    const change = vi.fn()
    f.render(h(StreamAudioSheet, { open: true, scopeId: 'audio', value: 5, onChange: change }))
    await settle()
    f.focus(f.find('audio.value'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Action)
    inputDispatcher.dispatch(NavigationIntent.Action)
    f.fireClick(f.find('audio.value.keyboard.done'), 'mouse')
    await settle()
    expect(change).toHaveBeenLastCalledWith(10)
    expect(f.engine.getCurrentFocus()?.id).toBe('audio.value')
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    f.fireClick(f.find('audio.value.keyboard.clear'), 'mouse')
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(change).toHaveBeenCalledOnce()
  })
})
