import { describe, expect, it } from 'vitest'
import { normalizeRenderControls } from '../shared/config/render-controls'
import { useSessionRenderControls } from './render-controls'

const initial = { render_mode: 'sr', display_options: { sharpness: 3, saturation: 105, contrast: 105, brightness: 100 } } as const
const soft = normalizeRenderControls({ render_mode: 'sharpen', display_options: { sharpness: 1, saturation: 96, contrast: 96, brightness: 102 } })

describe('本次渲染配置范围与临时覆盖', () => {
  it('预览取消恢复原值，本次应用与重连初始化继续保留覆盖和启动默认', () => {
    const state = useSessionRenderControls()
    state.initialize(initial)
    state.preview(soft)
    expect(state.active.value).toEqual(soft)
    expect(state.current.value.mode).toBe('sr')
    state.clearTransient()
    expect(state.active.value.mode).toBe('sr')
    state.apply(soft)
    state.initialize({ render_mode: 'video' })
    state.clearTransient()
    expect(state.active.value).toEqual(soft)
    expect(state.defaults.value.mode).toBe('sr')
    expect(state.source.value).toBe('session')
    state.restoreDefaults()
    expect(state.active.value).toEqual(normalizeRenderControls(initial))
    expect(state.source.value).toBe('default')
  })

  it('a/B 临时切换完整恢复模式、颜色、锐化及配置来源', () => {
    const state = useSessionRenderControls()
    state.initialize(initial)
    state.apply(soft)
    const restore = state.beginTemporary(normalizeRenderControls({ render_mode: 'sr' }))
    expect(state.active.value.mode).toBe('sr')
    expect(state.current.value).toEqual(soft)
    restore()
    expect(state.active.value).toEqual(soft)
    expect(state.source.value).toBe('session')
    expect(state.defaults.value).toEqual(normalizeRenderControls(initial))
  })

  it('旧对照恢复与旧预览清理按代际吸收，新操作继续生效', () => {
    const state = useSessionRenderControls()
    state.initialize(initial)
    const restore = state.beginTemporary(soft)
    state.apply(normalizeRenderControls({ render_mode: 'video' }))
    restore()
    expect(state.active.value.mode).toBe('video')
    const first = state.beginTemporary(soft)
    const second = state.beginTemporary(normalizeRenderControls({ render_mode: 'sr' }))
    first()
    expect(state.active.value.mode).toBe('sr')
    second()
    expect(state.active.value.mode).toBe('video')
  })

  it('显式默认保存后两层采用新默认，恢复默认也使用该值', () => {
    const state = useSessionRenderControls()
    state.initialize(initial)
    state.acceptSavedDefault(soft)
    state.preview(normalizeRenderControls({ render_mode: 'video' }))
    state.clearTransient()
    expect(state.active.value).toEqual(soft)
    expect(state.source.value).toBe('default')
    soft.displayOptions.sharpness = 3
    expect(state.defaults.value.displayOptions.sharpness).toBe(1)
    soft.displayOptions.sharpness = 1
  })
})
