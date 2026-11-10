import { describe, expect, it } from 'vitest'
import { normalizeRenderControls, projectRenderControls, selectSharpening, sharpeningChoice } from './render-controls'

describe('直通引擎与独立画面处理', () => {
  it.each([
    [{}, false, 0],
    [{ render_mode: 'video', super_resolution_experimental: true, display_options: { sharpness: 3 } }, false, 0],
    [{ render_mode: 'sharpen', display_options: { sharpness: 2 } }, false, 1],
    [{ render_mode: 'sr', display_options: { sharpness: 3 } }, true, 0],
    [{ super_resolution_experimental: true, display_options: { sharpness: 3 } }, true, 0],
    [{ render_compatibility_mode: false, super_resolution_experimental: true, display_options: { sharpness: 3 } }, true, 3],
  ] as const)('迁移旧意图并保留新组合 %j', (config, superResolution, sharpness) => {
    expect(normalizeRenderControls(config)).toMatchObject({ compatibilityMode: false, superResolution, displayOptions: { sharpness } })
  })

  it('sR、锐化和风格独立，兼容意图保留所有参数', () => {
    const sr = normalizeRenderControls({ render_compatibility_mode: false, super_resolution_experimental: true })
    const soft = selectSharpening(sr, 'soft')
    expect(soft.superResolution).toBe(true)
    expect(sharpeningChoice(soft)).toBe('soft')
    expect(selectSharpening(soft, 'off')).toMatchObject({ superResolution: true, displayOptions: { sharpness: 0 } })
    const combined = { ...soft, compatibilityMode: true, displayOptions: { ...soft.displayOptions, brightness: 102, contrast: 96, saturation: 96 } }
    const projected = projectRenderControls({ enableAudioControl: false, displayOptions: sr.displayOptions }, combined)
    expect(projected).toMatchObject({ pipelinePreference: 'video', superResolutionPreference: 'fsr1Experimental', displayOptions: combined.displayOptions })
    expect(projectRenderControls(projected, { ...combined, compatibilityMode: false })).toMatchObject({ pipelinePreference: 'webgl2', superResolutionPreference: 'fsr1Experimental', displayOptions: combined.displayOptions })
    expect(projectRenderControls(projected, normalizeRenderControls({}))).toMatchObject({ pipelinePreference: 'webgl2', superResolutionPreference: 'off', displayOptions: { sharpness: 0 } })
  })
})
