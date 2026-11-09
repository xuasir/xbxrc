import { computed, createApp, defineComponent, h, nextTick, ref } from 'vue'
import { applyTheme } from '@/app/theme'
import SettingInputControl from '@/components/settings/SettingInputControl.vue'
import StreamAudioSheet from '@/components/stream/StreamAudioSheet.vue'
import StreamTextSheet from '@/components/stream/StreamTextSheet.vue'
import { i18n, setUiLocale } from '@/i18n'
import { navigationEngine } from '@/navigation/core/engine'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import AlbumVideoPlayer from '@/pages/library/AlbumVideoPlayer.vue'
import LibraryRefresh from '@/pages/library/LibraryRefresh.vue'
import LibrarySelectControl from '@/pages/library/LibrarySelectControl.vue'
import SettingSectionList from '@/pages/settings/SettingSectionList.vue'
import sample from '../../2026-11-09-gamepad-focus-action-completion/sample.mp4'
import '@/styles/base.css'
import '@/styles/tokens.css'
import '@/styles/theme.scss'
import '@/pages/library/library.scss'
import './fixed.css'

const frame = () => new Promise(resolve => requestAnimationFrame(resolve))
async function settle() {
  await nextTick()
  await frame()
  await frame()
}
async function until(condition) {
  const deadline = performance.now() + 3000
  while (!condition()) {
    if (performance.now() > deadline)
      throw new Error('异步 UI 状态等待超时')
    await frame()
  }
}
const App = defineComponent({
  setup() {
    const scenario = ref('settings')
    const busy = ref(false)
    const disabled = ref(false)
    const enabled = ref(false)
    const text = ref('')
    const volume = ref(5)
    const results = ref([])
    const running = ref(false)
    const theme = ref('dark')
    const selects = ref({})
    const optionValues = [{ value: 'one', label: '选项一' }, { value: 'two', label: '选项二' }]
    const selectIds = ['console.games.storage', 'console.games.sort', 'my.friends.filter', 'game.achievements.filter', 'album.filter', 'album.game-filter', 'my.achievements.filter']
    let count = 0
    let operation
    const sections = computed(() => [{ key: 'settings', label: '保存期间保持来源', entries: [
      { kind: 'field', row: { key: 'toggle', nodeId: 'fixed.toggle', control: 'toggle', label: '自动保存开关', value: enabled.value, description: 'A 保存期间继续保持导航位置' } },
      { kind: 'field', row: { key: 'choice', nodeId: 'fixed.choice', control: 'singleSelect', label: '配置选项', value: 'one', options: optionValues } },
    ] }])
    function save() {
      if (busy.value)
        return
      count++
      busy.value = true
      operation = new Promise(resolve => setTimeout(() => {
        enabled.value = !enabled.value
        busy.value = false
        resolve()
      }, 600))
    }
    function snapshot() {
      return {
        current: navigationEngine.getCurrentFocus()?.id ?? '',
        native: document.activeElement?.id ?? '',
        marked: [...document.querySelectorAll('[data-focused="true"]')].map(element => element.id),
        busy: busy.value,
        count,
        topVideoScope: navigationEngine.isTopmostScope('album.preview.video'),
        nativeFullscreen: document.fullscreenElement?.id ?? '',
      }
    }
    function focus(id) {
      const target = document.getElementById(id)
      if (!target || !navigationEngine.focusElement(target, false, true))
        throw new Error(`焦点入口缺失：${id}`)
    }
    const action = () => inputDispatcher.dispatch(NavigationIntent.Action)
    const back = () => inputDispatcher.dispatch(NavigationIntent.Back)
    async function view(name) {
      scenario.value = name
      busy.value = disabled.value = false
      operation = undefined
      count = 0
      await settle()
    }
    async function check(name, test) {
      try {
        const evidence = await test()
        results.value.push({ name, passed: true, evidence })
      }
      catch (error) {
        results.value.push({ name, passed: false, error: error.message, evidence: snapshot() })
      }
    }
    function assert(condition, message) {
      if (!condition)
        throw new Error(message)
    }
    async function run() {
      running.value = true
      results.value = []
      await check('设置 A 保存保持焦点与重复确认合并', async () => {
        await view('settings')
        focus('fixed.toggle')
        await settle()
        action()
        await settle()
        const during = snapshot()
        action()
        action()
        assert(during.current === 'fixed.toggle' && during.marked.length === 1 && count === 1, '保存期间焦点或重复动作异常')
        await operation
        await settle()
        assert(snapshot().current === 'fixed.toggle', '保存结束焦点异常')
        return { during, after: snapshot() }
      })
      await check('保存期间方向移动保持用户新位置', async () => {
        await view('settings')
        focus('fixed.toggle')
        action()
        await settle()
        inputDispatcher.dispatch(NavigationIntent.Down)
        const moved = snapshot().current
        assert(moved !== 'fixed.toggle' && moved !== '', '方向移动缺失')
        await operation
        await settle()
        assert(snapshot().current === moved, '保存结束覆盖了新位置')
        return snapshot()
      })
      await check('刷新 A 保持来源并合并重复请求', async () => {
        await view('refresh')
        focus('fixed.refresh')
        action()
        await settle()
        action()
        const during = snapshot()
        await operation
        await settle()
        assert(count === 1 && during.current === 'fixed.refresh' && snapshot().current === 'fixed.refresh', '刷新焦点异常')
        return { during, after: snapshot() }
      })
      await check('真实 disabled 经原生 focus 后恢复来源', async () => {
        await view('disabled')
        focus('fixed.source')
        await settle()
        disabled.value = true
        await settle()
        const fallback = snapshot()
        disabled.value = false
        await settle()
        assert(fallback.current === 'fixed.safe' && snapshot().current === 'fixed.source', '原生 focus 后恢复异常')
        return { fallback, after: snapshot() }
      })
      for (const id of selectIds) await check(`${id} A / 方向 / 提交 / B`, async () => {
        await view('select')
        selects.value[id] = 'one'
        await settle()
        focus(id)
        action()
        await settle()
        assert(snapshot().current === `${id}.options.option.0`, '当前选项焦点异常')
        inputDispatcher.dispatch(NavigationIntent.Down)
        action()
        await settle()
        assert(selects.value[id] === 'two' && snapshot().current === id, '选中或来源恢复异常')
        action()
        await settle()
        back()
        await settle()
        assert(selects.value[id] === 'two' && snapshot().current === id, '取消或来源恢复异常')
        return { ...snapshot(), value: selects.value[id] }
      })
      await check('搜索共享键盘完成与 B 取消', async () => {
        await view('input')
        text.value = ''
        focus('xcloud.search')
        action()
        await settle()
        assert(snapshot().current === 'xcloud.search.keyboard.key.0.0', '键盘未取得焦点')
        action()
        focus('xcloud.search.keyboard.done')
        action()
        await settle()
        assert(text.value === '1' && snapshot().current === 'xcloud.search', '键盘回填异常')
        action()
        await settle()
        focus('xcloud.search.keyboard.clear')
        action()
        back()
        await settle()
        assert(text.value === '1' && snapshot().current === 'xcloud.search', '取消覆盖了父草稿')
        return { ...snapshot(), text: text.value }
      })
      await check('串流文字嵌套键盘与发送去重', async () => {
        await view('text')
        focus('fixed.text.field')
        action()
        await settle()
        action()
        focus('fixed.text.field.keyboard.done')
        action()
        await settle()
        assert(snapshot().current === 'fixed.text.field' && count === 0, '键盘完成误提交父表单')
        focus('fixed.text.submit')
        action()
        await settle()
        action()
        assert(count === 1 && snapshot().current === 'fixed.text.submit', '发送期间重复动作或移焦')
        await operation
        await settle()
        return snapshot()
      })
      await check('串流音量数字键盘与 0–10 边界', async () => {
        await view('audio')
        focus('fixed.audio.value')
        action()
        await settle()
        action()
        action()
        focus('fixed.audio.value.keyboard.done')
        action()
        await settle()
        assert(volume.value === 10 && snapshot().current === 'fixed.audio.value', '数字键盘或数值边界异常')
        return { ...snapshot(), volume: volume.value }
      })
      await check('实际视频进度 / 音量 / 静音 / 全屏返回', async () => {
        await view('video')
        const video = document.querySelector('video')
        if (video.readyState === 0) await new Promise((resolve, reject) => {
          video.addEventListener('loadedmetadata', resolve, { once: true })
          video.addEventListener('error', () => reject(new Error('本地夹具视频加载失败')), { once: true })
          setTimeout(() => reject(new Error('元数据超时')), 5000)
        })
        await settle()
        focus('album.preview.forward')
        action()
        action()
        action()
        await settle()
        assert(video.currentTime === video.duration, '进度上界异常')
        focus('album.preview.rewind')
        action()
        action()
        action()
        focus('album.preview.volume-down')
        for (let i = 0; i < 12; i++) action()
        assert(video.currentTime === 0 && video.volume === 0, '进度或音量下界异常')
        focus('album.preview.volume-up')
        for (let i = 0; i < 12; i++) action()
        focus('album.preview.mute')
        action()
        assert(video.volume === 1 && video.muted, '音量或静音异常')
        focus('album.preview.play')
        action()
        await settle()
        assert(!video.paused, '播放确认异常')
        action()
        await settle()
        focus('album.preview.fullscreen')
        action()
        await settle()
        await until(() => navigationEngine.isTopmostScope('album.preview.video') && document.getElementById('album.preview.fullscreen').dataset.navBusy !== 'true')
        const fullscreen = snapshot()
        back()
        await until(() => navigationEngine.isTopmostScope('app.shell') && document.activeElement?.id === 'album.preview.fullscreen')
        await settle()
        assert(snapshot().current === 'album.preview.fullscreen' && snapshot().native === 'album.preview.fullscreen' && navigationEngine.isTopmostScope('app.shell'), '全屏退出来源异常')
        return { fullscreen, after: snapshot(), duration: video.duration, time: video.currentTime, volume: video.volume }
      })
      await view('settings')
      focus('fixed.toggle')
      running.value = false
    }
    function content() {
      if (scenario.value === 'settings')
        return h(SettingSectionList, { sections: sections.value, scopeId: 'app.shell', pendingActionKey: busy.value ? 'save' : null, expanded: new Set(), onRowConfirm: save, onRowSelect: save })
      if (scenario.value === 'refresh')
        return h('div', { class: 'library-toolbar' }, [h(LibraryRefresh, { id: 'fixed.refresh', busy: busy.value, onRefresh: save }), h(Focusable, { id: 'fixed.safe', as: 'button', class: 'library-control' }, () => '其他操作')])
      if (scenario.value === 'disabled')
        return h('div', { class: 'library-toolbar' }, [h(Focusable, { id: 'fixed.source', as: 'button', class: 'library-control', disabled: disabled.value }, () => '临时禁用来源'), h(Focusable, { id: 'fixed.safe', as: 'button', class: 'library-control' }, () => '回退节点')])
      if (scenario.value === 'select')
        return h('div', { class: 'library-toolbar' }, selectIds.map(id => h(LibrarySelectControl, { id, label: id, modelValue: selects.value[id] ?? 'one', options: optionValues, 'onUpdate:modelValue': value => selects.value[id] = value })))
      if (scenario.value === 'input')
        return h(SettingInputControl, { id: 'xcloud.search', scopeId: 'app.shell', label: '搜索游戏', type: 'search', modelValue: text.value, placeholder: '搜索游戏', 'onUpdate:modelValue': value => text.value = value })
      if (scenario.value === 'text')
        return h(StreamTextSheet, { open: true, scopeId: 'fixed.text', loading: busy.value, onClose: () => scenario.value = 'settings', onSubmit: save })
      if (scenario.value === 'audio')
        return h(StreamAudioSheet, { open: true, scopeId: 'fixed.audio', value: volume.value, onChange: value => volume.value = value, onClose: () => scenario.value = 'settings' })
      if (scenario.value === 'video')
        return h(AlbumVideoPlayer, { src: sample })
    }
    return () => h('div', { class: 'verification-page' }, [
      h('header', { class: 'verification-header' }, [h('h1', '焦点导航修复验证'), h('span', { id: 'verification-summary', role: 'status' }, running.value ? '验证进行中' : `${results.value.filter(row => row.passed).length} / ${results.value.length} 场景通过`)]),
      h('p', '生产 Vue 控件、共享几何导航与本地视频；A 经公共 inputDispatcher 注入。'),
      h('div', { class: 'verification-controls' }, [h('button', { disabled: running.value, onClick: run }, '运行全部验证'), h('button', { onClick: () => { theme.value = theme.value === 'dark' ? 'light' : 'dark'; applyTheme(theme.value) } }, theme.value === 'dark' ? '切换浅色' : '切换深色'), ...['settings', 'refresh', 'select', 'input', 'text', 'audio', 'video'].map(name => h('button', { disabled: running.value, onClick: () => view(name) }, name))]),
      h(FocusScope, { id: 'app.shell', as: 'main', class: 'verification-area', restoreFocus: false }, () => h('section', { 'data-nav-page-active': 'true' }, [content()])),
      h('details', [h('summary', '验证记录'), h('pre', { id: 'verification-results' }, JSON.stringify(results.value, null, 2))]),
    ])
  },
})
setUiLocale('zh')
applyTheme('dark')
const density = () => document.documentElement.dataset.uiDensity = window.innerWidth <= 720 ? 'narrow' : 'comfortable'
density()
window.addEventListener('resize', density)
navigationEngine.start()
createApp(App).use(i18n).mount('#app')
