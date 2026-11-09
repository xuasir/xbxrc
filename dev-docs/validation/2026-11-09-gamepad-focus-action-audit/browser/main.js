import { computed, createApp, defineComponent, h, nextTick, ref } from 'vue'
import { createI18n } from 'vue-i18n'
import SettingInputControl from '@/components/settings/SettingInputControl.vue'
import { navigationEngine } from '@/navigation/core/engine'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import LibraryRefresh from '@/pages/library/LibraryRefresh.vue'
import SettingSectionList from '@/pages/settings/SettingSectionList.vue'
import '@/styles/base.css'

const labels = [
  ['ordinary', '普通按钮 A'],
  ['focusable', 'Focusable 回退 A'],
  ['raw', '原生节点回退 A'],
  ['settings', '设置整组保存 A'],
  ['refresh', '刷新按钮 A'],
  ['select', '原生下拉框 A'],
  ['cloudSearch', '云游戏搜索 A'],
  ['settingInput', '设置输入 A'],
]
const frame = () => new Promise(resolve => requestAnimationFrame(resolve))
async function frames(count = 2) {
  for (let i = 0; i < count; i++) await frame()
}
const App = defineComponent({
  setup() {
    const scenario = ref('ordinary')
    const busy = ref(null)
    const toggle = ref(false)
    const choice = ref('one')
    const text = ref('')
    const results = ref([])
    const running = ref(false)
    let timeline = []
    let operation
    let clicks = 0
    let lastClickTrusted = null
    let phase = 'idle'
    function snapshot(stage) {
      const select = document.getElementById('audit.select')
      let pickerOpen = null
      try {
        pickerOpen = select?.matches(':open') ?? null
      }
      catch { /* 旧引擎通过 AX 树核对。 */ }
      return {
        stage,
        current: navigationEngine.getCurrentFocus()?.id ?? '',
        nativeFocus: document.activeElement?.id ?? '',
        marked: [...document.querySelectorAll('[data-focused="true"]')].map(element => element.id),
        pendingRecovery: navigationEngine.pendingRecovery?.id ?? '',
        busy: busy.value,
        pickerOpen,
        selectValue: select?.value,
        dialogs: [...document.querySelectorAll('[role="dialog"]')].map(element => element.id),
      }
    }
    navigationEngine.onFocusChange(element => timeline.push({ event: 'engineFocus', phase, id: element?.id ?? '' }))
    document.addEventListener('focusin', (event) => {
      timeline.push({ event: 'nativeFocus', phase, id: event.target.id, pendingRecovery: navigationEngine.pendingRecovery?.id ?? '' })
    })
    document.addEventListener('click', (event) => {
      if (event.target.id?.startsWith('audit.'))
        lastClickTrusted = event.isTrusted
    }, true)
    const sections = computed(() => [{ key: 'options', label: '配置保存', entries: [
      { kind: 'field', row: { key: 'toggle', nodeId: 'audit.setting.toggle', control: 'toggle', label: '自动保存开关', value: toggle.value } },
      { kind: 'field', row: { key: 'choice', nodeId: 'audit.setting.choice', control: 'singleSelect', label: '自动保存单选', options: [{ value: 'one', label: '一' }, { value: 'two', label: '二' }], value: choice.value } },
    ] }])
    function save(key, value) {
      clicks++
      operation = (async () => {
        busy.value = key
        phase = 'busy'
        await nextTick()
        timeline.push(snapshot('afterDisabledMutation'))
        await frames(3)
        timeline.push(snapshot('afterFallbackNativeFocus'))
        if (key === 'toggle')
          toggle.value = value
        if (key === 'choice')
          choice.value = value
        busy.value = null
        phase = 'saved'
        await nextTick()
        await frames(3)
      })()
    }
    async function run(name) {
      running.value = true
      navigationEngine.clearFocus()
      navigationEngine.forgetFocus('')
      scenario.value = name
      busy.value = null
      operation = undefined
      clicks = 0
      lastClickTrusted = null
      await nextTick()
      await frames(2)
      const id = name === 'settings' ? 'audit.setting.toggle' : name === 'refresh' ? 'audit.refresh' : name === 'select' ? 'audit.select' : name === 'cloudSearch' || name === 'settingInput' ? 'audit.input' : 'audit.source'
      navigationEngine.focusElement(document.getElementById(id), false, true)
      await frames(2)
      timeline = []
      phase = 'action'
      const before = snapshot('beforeA')
      inputDispatcher.dispatch(NavigationIntent.Action)
      if (operation)
        await operation
      await nextTick()
      await frames(3)
      const result = { scenario: name, clicks, lastClickTrusted, before, after: snapshot('afterA'), timeline: [...timeline] }
      results.value.push(result)
      running.value = false
      phase = 'idle'
    }
    function content() {
      const name = scenario.value
      const safe = name === 'raw'
        ? h('button', { 'id': 'audit.safe', 'data-focusable': 'true' }, '原生回退节点')
        : h(Focusable, { id: 'audit.safe', as: 'button' }, () => '回退按钮')
      if (name === 'settings')
        return h('div', { class: 'audit-settings' }, [h(Focusable, { id: 'audit.sidebar', as: 'button' }, () => '通用分类'), h(SettingSectionList, { sections: sections.value, scopeId: 'app.shell', pendingActionKey: busy.value, expanded: new Set(), onRowConfirm: row => save(row.key, !row.value), onRowSelect: (row, value) => save(row.key, value) })])
      let source
      if (name === 'refresh')
        source = h(LibraryRefresh, { id: 'audit.refresh', busy: busy.value !== null, onRefresh: () => save('refresh') })
      else if (name === 'select')
        source = h(Focusable, { 'id': 'audit.select', 'as': 'select', 'value': choice.value, 'aria-label': '原生筛选', 'onClick': () => { clicks++ }, 'onChange': event => choice.value = event.target.value }, () => [h('option', { value: 'one' }, '选项一'), h('option', { value: 'two' }, '选项二')])
      else if (name === 'cloudSearch')
        source = h('input', { 'id': 'audit.input', 'type': 'search', 'data-focusable': 'true', 'aria-label': '云游戏搜索', 'onFocus': event => navigationEngine.focusElement(event.target, false) })
      else if (name === 'settingInput')
        source = h(SettingInputControl, { 'id': 'audit.input', 'scopeId': 'app.shell', 'label': '设置输入', 'modelValue': text.value, 'onUpdate:modelValue': value => text.value = value })
      else
        source = h(Focusable, { id: 'audit.source', as: 'button', disabled: busy.value !== null, onClick: () => name === 'ordinary' ? clicks++ : save('source') }, () => '确认来源')
      return h('div', { class: 'audit-pair' }, [source, safe])
    }
    return () => h('div', [
      h('h1', '焦点确认时序审计'),
      h('p', '实际 Vue 控件与几何导航；保存、输入来源和反馈使用本地夹具。'),
      h('div', { class: 'audit-controls' }, labels.map(([name, label]) => h('button', { disabled: running.value, onClick: () => run(name) }, label))),
      h(FocusScope, { id: 'app.shell', as: 'main', class: 'app-shell', restoreFocus: false }, () => h('section', { 'data-nav-page-active': 'true', 'class': 'audit-page' }, [h('p', `当前场景：${scenario.value}`), content()])),
      h('pre', { id: 'audit-results' }, JSON.stringify(results.value, null, 2)),
    ])
  },
})
navigationEngine.start()
createApp(App).use(createI18n({ legacy: false, locale: 'zh-CN', messages: { 'zh-CN': {} }, missingWarn: false, fallbackWarn: false })).mount('#app')
