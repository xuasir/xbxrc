import type { StreamingDisplayOptionsValue, StreamingRenderProjection } from '../rpc/streaming'

export type SharpeningChoice = 'off' | 'clear' | 'soft'
export type ColorPreset = 'standard' | 'clear' | 'soft'

export interface RenderControlsValue {
  compatibilityMode: boolean
  superResolution: boolean
  displayOptions: StreamingDisplayOptionsValue
}

export const SHARPENING_STRENGTH = { soft: 1, clear: 3 } as const
export const COLOR_PRESETS = {
  standard: { saturation: 100, contrast: 100, brightness: 100 },
  clear: { saturation: 105, contrast: 105, brightness: 100 },
  soft: { saturation: 96, contrast: 96, brightness: 102 },
} as const

export function normalizeRenderControls(config: {
  render_mode?: unknown
  render_compatibility_mode?: unknown
  super_resolution_experimental?: unknown
  display_options?: Partial<StreamingDisplayOptionsValue> | null
}): RenderControlsValue {
  const value = config.display_options
  // 新合同的兼容键标记独立效果；旧模式仅在首次迁移时解释记忆锐度。
  const legacy = config.render_compatibility_mode === undefined
  const mode = legacy ? config.render_mode : undefined
  const superResolution = mode === 'video' || mode === 'sharpen' ? false : mode === 'sr' || config.super_resolution_experimental === true
  const rawSharpness = legacy && (mode === 'video' || mode === 'sr' || (mode === undefined && superResolution)) ? 0 : value?.sharpness ?? 0
  const color = (raw: number | undefined): number => raw !== undefined && Number.isFinite(raw) ? Math.min(200, Math.max(0, Math.round(raw))) : 100
  return {
    compatibilityMode: config.render_compatibility_mode === true,
    superResolution,
    displayOptions: {
      sharpness: rawSharpness > 0 ? rawSharpness <= 2 ? SHARPENING_STRENGTH.soft : SHARPENING_STRENGTH.clear : 0,
      saturation: color(value?.saturation),
      contrast: color(value?.contrast),
      brightness: color(value?.brightness),
    },
  }
}

export function copyRenderControls(value: RenderControlsValue): RenderControlsValue {
  return { compatibilityMode: value.compatibilityMode, superResolution: value.superResolution, displayOptions: { ...value.displayOptions } }
}

export function sharpeningChoice(value: RenderControlsValue): SharpeningChoice {
  return value.displayOptions.sharpness > 0 ? value.displayOptions.sharpness <= 2 ? 'soft' : 'clear' : 'off'
}

export function selectSharpening(value: RenderControlsValue, choice: SharpeningChoice): RenderControlsValue {
  return {
    ...value,
    displayOptions: { ...value.displayOptions, sharpness: choice === 'off' ? 0 : SHARPENING_STRENGTH[choice] },
  }
}

/** 当前会话以完整模式覆盖启动投影，明确关闭与颜色调整保持一致。 */
export function projectRenderControls(render: StreamingRenderProjection, value: RenderControlsValue): StreamingRenderProjection {
  return {
    ...render,
    displayOptions: { ...value.displayOptions },
    pipelinePreference: value.compatibilityMode ? 'video' : 'webgl2',
    superResolutionPreference: value.superResolution ? 'fsr1Experimental' : 'off',
  }
}

export function renderControlsPatch(value: RenderControlsValue) {
  return { render_compatibility_mode: value.compatibilityMode, super_resolution_experimental: value.superResolution, display_options: { ...value.displayOptions } }
}

export function renderControlsLabel(value: RenderControlsValue, t: (key: string) => string): string {
  if (value.compatibilityMode)
    return t('setting.renderControls.compatibilityRendering')
  const parts: string[] = []
  if (value.superResolution)
    parts.push('SR')
  if (sharpeningChoice(value) !== 'off')
    parts.push(`${t('setting.renderControls.sharpening')}${t(`setting.renderControls.choices.${sharpeningChoice(value)}`)}`)
  const style = (Object.keys(COLOR_PRESETS) as ColorPreset[]).find(key => Object.entries(COLOR_PRESETS[key]).every(([name, number]) => value.displayOptions[name as keyof StreamingDisplayOptionsValue] === number))
  if (style !== 'standard')
    parts.push(t(`setting.renderControls.colors.${style ?? 'custom'}`))
  return parts.length ? parts.join(' · ') : t('setting.renderControls.defaultRendering')
}
