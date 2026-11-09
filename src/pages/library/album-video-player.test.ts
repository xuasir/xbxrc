import type { FocusNode } from '@/navigation/core/__tests__/focus-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, nextTick, ref } from 'vue'
import { enableClientTemplate, vueFocusFixture } from '@/navigation/core/__tests__/vue-focus'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import AlbumVideoPlayer from './AlbumVideoPlayer.vue'

vi.mock('@/navigation/core/haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
enableClientTemplate(Focusable, new URL('../../navigation/core/vue/Focusable.vue', import.meta.url))
enableClientTemplate(FocusScope, new URL('../../navigation/core/vue/FocusScope.vue', import.meta.url))
enableClientTemplate(AlbumVideoPlayer, new URL('./AlbumVideoPlayer.vue', import.meta.url))
const fixtures: ReturnType<typeof vueFocusFixture>[] = []
async function settle() {
  for (let i = 0; i < 4; i++) await nextTick()
}
function fixture() {
  const f = vueFocusFixture()
  fixtures.push(f)
  vi.stubGlobal('navigator', { userActivation: { isActive: false } })
  const src = ref('first.mp4')
  const busy = ref(false)
  const operationError = vi.fn()
  const mediaError = vi.fn()
  f.render(h(defineComponent(() => () => h(FocusScope, { id: 'album.preview' }, () => [
    h(AlbumVideoPlayer, { src: src.value, busy: busy.value, onOperationError: operationError, onError: mediaError }),
    h(Focusable, { id: 'outside', as: 'button' }, () => '保存'),
  ]))))
  const element = () => document.querySelector('video') as unknown as FocusNode & HTMLVideoElement
  const press = (id: string) => {
    f.focus(f.find(`album.preview.${id}`))
    inputDispatcher.dispatch(NavigationIntent.Action)
  }
  return { ...f, src, busy, operationError, mediaError, element, press }
}
afterEach(() => {
  for (const f of fixtures.splice(0)) f.dispose()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('相册视频的手柄操作', () => {
  it('元数据到达后提供进度操作，连续 A 在 0–duration 内前后跳转', async () => {
    const f = fixture()
    await settle()
    expect(f.find('album.preview.forward').disabled).toBe(true)
    const video = f.element()
    Object.assign(video, { duration: 25, currentTime: 3 })
    f.fire(video, 'loadedmetadata')
    await settle()
    f.press('rewind')
    expect(video.currentTime).toBe(0)
    f.press('forward')
    f.press('forward')
    f.press('forward')
    expect(video.currentTime).toBe(25)
    expect(f.engine.getCurrentFocus()?.id).toBe('album.preview.forward')
  })

  it('音量限制 0–1，静音与提高音量共用当前 video 节点', async () => {
    const f = fixture()
    await settle()
    const video = f.element()
    video.volume = 0.05
    f.fire(video, 'volumechange')
    f.press('volume-down')
    expect(video.volume).toBe(0)
    for (let i = 0; i < 12; i++) f.press('volume-up')
    expect(video.volume).toBe(1)
    f.press('mute')
    expect(video.muted).toBe(true)
    f.press('volume-up')
    expect(video.muted).toBe(false)
    expect(video.volume).toBe(1)
  })

  it('播放请求期间合并重复 A，失败给出反馈并保留来源', async () => {
    const f = fixture()
    await settle()
    let reject = (_error: Error) => {}
    vi.mocked(f.element().play).mockImplementation(() => new Promise((_, fail) => reject = fail))
    f.press('play')
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(f.element().play).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('album.preview.play')
    reject(new Error('播放失败'))
    await settle()
    expect(f.operationError).toHaveBeenCalledOnce()
    expect(f.find('album.preview.play').attrs.has('data-nav-busy')).toBe(false)
  })

  it.each(['fulfilled', 'rejected'])('uRL 重建后的旧播放请求 %s，反馈与播放归属保持当前节点', async (outcome) => {
    const f = fixture()
    await settle()
    const old = f.element()
    let resolve = () => {}
    let reject = (_error: Error) => {}
    vi.mocked(old.play).mockImplementation(() => new Promise((done, fail) => {
      resolve = done
      reject = fail
    }))
    f.press('play')
    f.src.value = 'second.mp4'
    await settle()
    const current = f.element()
    expect(current).not.toBe(old)
    f.fire(old, 'error')
    if (outcome === 'fulfilled') {
      Object.assign(old, { paused: false })
      resolve()
    }
    else {
      reject(new Error('旧请求失败'))
    }
    await settle()
    expect(current.paused).toBe(true)
    expect(f.operationError).not.toHaveBeenCalled()
    expect(f.mediaError).not.toHaveBeenCalled()
    if (outcome === 'fulfilled')
      expect(old.paused).toBe(true)
    f.press('play')
    await settle()
    expect(current.paused).toBe(false)
    f.press('play')
    expect(current.paused).toBe(true)
  })

  it('手柄进入窗口内全屏，Scope 限定操作区，B 退出并恢复全屏按钮', async () => {
    const f = fixture()
    await settle()
    f.press('fullscreen')
    await settle()
    expect(f.engine.isTopmostScope('album.preview.video')).toBe(true)
    expect(f.find('album.preview.fullscreen').attrs.get('aria-pressed')).toBe('true')
    expect(f.engine.focusElement(f.find('outside') as unknown as HTMLElement, false)).toBe(false)
    f.busy.value = true
    await settle()
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(f.engine.isTopmostScope('album.preview')).toBe(true)
    expect(f.engine.getCurrentFocus()?.id).toBe('album.preview.fullscreen')
    expect(f.find('album.preview.fullscreen').attrs.has('aria-pressed')).toBe(false)
  })

  it.each(['success', 'denied'])('原生全屏 %s 后继续提供手柄退出', async (outcome) => {
    const f = fixture()
    await settle()
    Object.assign(navigator, { userActivation: { isActive: true } })
    Object.assign(document, { fullscreenEnabled: true })
    const container = f.find('album.preview.video')
    const request = vi.fn(async () => {
      if (outcome === 'denied')
        throw new Error('激活限制')
      Object.assign(document, { fullscreenElement: container })
    })
    const exit = vi.fn(async () => {
      Object.assign(document, { fullscreenElement: null })
    })
    Object.assign(container, { requestFullscreen: request })
    Object.assign(document, { exitFullscreen: exit })
    f.press('fullscreen')
    await settle()
    expect(request).toHaveBeenCalledOnce()
    expect(f.engine.isTopmostScope('album.preview.video')).toBe(true)
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(f.engine.isTopmostScope('album.preview')).toBe(true)
    expect(exit).toHaveBeenCalledTimes(outcome === 'success' ? 1 : 0)
    expect(f.operationError).not.toHaveBeenCalled()
  })

  it.each([false, true])('退出全屏末帧同步 DOM 焦点，后续用户移动 %s 决定最终位置', async (moved) => {
    const f = fixture()
    await settle()
    f.press('fullscreen')
    await settle()
    const frames: Array<() => void> = []
    vi.stubGlobal('requestAnimationFrame', (callback: () => void) => frames.push(callback))
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    f.find('album.preview.fullscreen').blur()
    if (moved)
      f.focus(f.find('outside'))
    for (const callback of frames.splice(0)) callback()
    expect(f.active()?.id).toBe(moved ? 'outside' : 'album.preview.fullscreen')
    expect(f.engine.getCurrentFocus()?.id).toBe(moved ? 'outside' : 'album.preview.fullscreen')
  })

  it('进入原生全屏的 Promise 等待期间，B 撤销旧请求并退出', async () => {
    const f = fixture()
    await settle()
    Object.assign(navigator, { userActivation: { isActive: true } })
    Object.assign(document, { fullscreenEnabled: true })
    const container = f.find('album.preview.video')
    const changed = vi.mocked(document.addEventListener).mock.calls.find(call => call[0] === 'fullscreenchange')![1] as () => void
    let resolve = () => {}
    Object.assign(container, { requestFullscreen: vi.fn(() => new Promise<void>((done) => {
      Object.assign(document, { fullscreenElement: container })
      changed()
      resolve = done
    })) })
    const exit = vi.fn(async () => {
      Object.assign(document, { fullscreenElement: null })
      changed()
    })
    Object.assign(document, { exitFullscreen: exit })
    f.press('fullscreen')
    await settle()
    expect(f.find('album.preview.fullscreen').attrs.get('data-nav-busy')).toBe('true')
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    resolve()
    await settle()
    expect(exit).toHaveBeenCalledOnce()
    expect(f.engine.isTopmostScope('album.preview')).toBe(true)
    expect(f.active()?.id).toBe('album.preview.fullscreen')
  })

  it('webKit 容器全屏能力提供相同的操作区与 B 退出', async () => {
    const f = fixture()
    await settle()
    Object.assign(navigator, { userActivation: { isActive: true } })
    const container = f.find('album.preview.video')
    const request = vi.fn(() => {
      Object.assign(document, { webkitFullscreenElement: container })
    })
    const exit = vi.fn(() => {
      Object.assign(document, { webkitFullscreenElement: null })
    })
    Object.assign(container, { webkitRequestFullscreen: request })
    Object.assign(document, { webkitExitFullscreen: exit })
    f.press('fullscreen')
    await settle()
    expect(request).toHaveBeenCalledOnce()
    inputDispatcher.dispatch(NavigationIntent.Back)
    await settle()
    expect(exit).toHaveBeenCalledOnce()
    expect(f.engine.isTopmostScope('album.preview')).toBe(true)
  })

  it('卸载期间完成的原生全屏请求释放自身容器并清理监听', async () => {
    const f = fixture()
    await settle()
    Object.assign(navigator, { userActivation: { isActive: true } })
    Object.assign(document, { fullscreenEnabled: true })
    const container = f.find('album.preview.video')
    let resolve = () => {}
    Object.assign(container, { requestFullscreen: vi.fn(() => new Promise<void>((done) => {
      resolve = () => {
        Object.assign(document, { fullscreenElement: container })
        done()
      }
    })) })
    const exit = vi.fn(async () => {
      Object.assign(document, { fullscreenElement: null })
    })
    Object.assign(document, { exitFullscreen: exit })
    f.press('fullscreen')
    f.render(null)
    resolve()
    await settle()
    expect(exit).toHaveBeenCalledOnce()
    expect(document.fullscreenElement).toBeNull()
    expect(document.removeEventListener).toHaveBeenCalledWith('fullscreenchange', expect.any(Function))
    expect(f.operationError).not.toHaveBeenCalled()
  })

  it('进度操作失败给出反馈，busy 保持视频操作候选并合并确认', async () => {
    const f = fixture()
    await settle()
    const video = f.element()
    Object.assign(video, { duration: 50 })
    f.fire(video, 'loadedmetadata')
    await settle()
    Object.defineProperty(video, 'currentTime', { get: () => 0, set: () => {
      throw new Error('进度不可用')
    } })
    f.press('forward')
    expect(f.operationError).toHaveBeenCalledOnce()
    f.busy.value = true
    await settle()
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(f.operationError).toHaveBeenCalledOnce()
    expect(f.engine.getCurrentFocus()?.id).toBe('album.preview.forward')
  })
})
