import { describe, expect, it } from 'vitest'
import { normalizeRenderControls, projectRenderControls, selectSharpening, sharpeningChoice } from './render-controls'

describe('互斥渲染控制与旧配置兼容', () => {
  it.each([
    [{}, 'video', 3],
    [{ display_options: { sharpness: 0 } }, 'video', 3],
    [{ display_options: { sharpness: 1 } }, 'sharpen', 1],
    [{ display_options: { sharpness: 2 } }, 'sharpen', 1],
    [{ display_options: { sharpness: 3 } }, 'sharpen', 3],
    [{ display_options: { sharpness: 10 } }, 'sharpen', 3],
    [{ super_resolution_experimental: true, display_options: { sharpness: 3 } }, 'sr', 3],
    [{ render_mode: 'video', super_resolution_experimental: true, display_options: { sharpness: 3 } }, 'video', 3],
  ] as const)('按原始设置迁移 %j', (config, mode, sharpness) => {
    expect(normalizeRenderControls(config)).toMatchObject({ mode, displayOptions: { sharpness } })
  })

  it('sR 与清晰/柔和互斥，关闭参数保留且明确关闭可覆盖后端 SR', () => {
    const sr = normalizeRenderControls({ render_mode: 'sr' })
    expect(sharpeningChoice(sr)).toBe('off')
    expect(selectSharpening(sr, 'off').mode).toBe('sr')
    const soft = selectSharpening(sr, 'soft')
    expect(sharpeningChoice(soft)).toBe('soft')
    const off = selectSharpening(soft, 'off')
    expect(off).toMatchObject({ mode: 'video', displayOptions: { sharpness: 1 } })
    expect(projectRenderControls({ enableAudioControl: false, displayOptions: sr.displayOptions, superResolutionPreference: 'fsr1Experimental' }, off)).toMatchObject({
      pipelinePreference: 'video',
      superResolutionPreference: 'off',
      displayOptions: { sharpness: 0 },
    })
  })

  it('颜色变化继续走明确所选默认渲染，独立锐化配置只在锐化模式执行', () => {
    const controls = normalizeRenderControls({ render_mode: 'video', display_options: { sharpness: 3, brightness: 102, contrast: 96, saturation: 96 } })
    const base = { enableAudioControl: false, displayOptions: controls.displayOptions }
    expect(projectRenderControls(base, controls)).toMatchObject({ pipelinePreference: 'video', displayOptions: { sharpness: 0, brightness: 102 } })
    expect(projectRenderControls(base, selectSharpening(controls, 'clear'))).toMatchObject({ pipelinePreference: 'webgl2', superResolutionPreference: 'off', displayOptions: { sharpness: 3 } })
    expect(projectRenderControls(base, { ...controls, mode: 'sr' })).toMatchObject({ pipelinePreference: 'webgl2', superResolutionPreference: 'fsr1Experimental', displayOptions: { sharpness: 0 } })
  })
})
