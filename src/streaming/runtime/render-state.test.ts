import { describe, expect, it } from 'vitest'
import zh from '../../i18n/locales/zh.json'
import { observeRenderState, renderStateLabel } from './render-state'

function t(key: string): string {
  return key.split('.').reduce<any>((value, part) => value[part], zh)
}

describe('实际渲染与最简顶部状态', () => {
  it('准备计划保留待提交标记，已提交管线承担实际模式', () => {
    expect(renderStateLabel(observeRenderState({ pipelineType: 'webgl2_sr', pendingRenderers: 1 }), t)).toBe('准备中')
    expect(renderStateLabel(observeRenderState({ actualPipeline: 'video', pendingRenderers: 1 }), t)).toBe('默认渲染 · 切换中')
    expect(renderStateLabel(observeRenderState({ actualPipeline: 'webgl2_sr', pendingRenderers: 0 }), t)).toBe('SR')
    expect(renderStateLabel(observeRenderState({ actualPipeline: 'webgl2', actualSharpness: 3 }), t)).toBe('锐化 · 清晰')
    expect(renderStateLabel(observeRenderState({ actualPipeline: 'webgl2', actualSharpness: 1 }), t)).toBe('锐化 · 柔和')
  })

  it.each([['performance', '帧率保护'], ['unsupported', '设备限制'], ['failed', '增强失败']])('默认渲染显示 %s 的简短原因', (reason, label) => {
    expect(renderStateLabel(observeRenderState({ actualPipeline: 'video', actualFallbackReason: reason }), t)).toBe(`默认渲染 · ${label}`)
  })

  it('释放后显示准备中，用户主动选择默认渲染显示确定模式', () => {
    expect(renderStateLabel(observeRenderState({ currentRenderers: 0 }), t)).toBe('准备中')
    expect(renderStateLabel(observeRenderState({ actualPipeline: 'video' }), t)).toBe('默认渲染')
  })
})
