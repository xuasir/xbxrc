import type { StreamingDisplayOptionsValue, StreamingRenderMode, StreamingRenderProjection } from '../rpc/streaming'

export type RenderMode = StreamingRenderMode
export type SharpeningChoice = 'off' | 'clear' | 'soft'
export type ColorPreset = 'standard' | 'clear' | 'soft'

export interface RenderControlsValue {
  mode: RenderMode
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
  super_resolution_experimental?: unknown
  display_options?: Partial<StreamingDisplayOptionsValue> | null
}): RenderControlsValue {
  const value = config.display_options
  const mode = config.render_mode === 'video' || config.render_mode === 'sharpen' || config.render_mode === 'sr'
    ? config.render_mode
    : config.super_resolution_experimental === true ? 'sr' : (value?.sharpness ?? 0) > 0 ? 'sharpen' : 'video'
  const color = (raw: number | undefined): number => raw !== undefined && Number.isFinite(raw) ? Math.min(200, Math.max(0, Math.round(raw))) : 100
  return {
    mode,
    displayOptions: {
      sharpness: value?.sharpness !== undefined && value.sharpness > 0 && value.sharpness <= 2 ? SHARPENING_STRENGTH.soft : SHARPENING_STRENGTH.clear,
      saturation: color(value?.saturation),
      contrast: color(value?.contrast),
      brightness: color(value?.brightness),
    },
  }
}

export function copyRenderControls(value: RenderControlsValue): RenderControlsValue {
  return { mode: value.mode, displayOptions: { ...value.displayOptions } }
}

export function sharpeningChoice(value: RenderControlsValue): SharpeningChoice {
  return value.mode === 'sharpen' ? value.displayOptions.sharpness <= 2 ? 'soft' : 'clear' : 'off'
}

export function selectSharpening(value: RenderControlsValue, choice: SharpeningChoice): RenderControlsValue {
  return {
    mode: choice === 'off' ? value.mode === 'sharpen' ? 'video' : value.mode : 'sharpen',
    displayOptions: { ...value.displayOptions, sharpness: choice === 'off' ? value.displayOptions.sharpness : SHARPENING_STRENGTH[choice] },
  }
}

/** 当前会话以完整模式覆盖启动投影，明确关闭与颜色调整保持一致。 */
export function projectRenderControls(render: StreamingRenderProjection, value: RenderControlsValue): StreamingRenderProjection {
  return {
    ...render,
    displayOptions: { ...value.displayOptions, sharpness: value.mode === 'sharpen' ? value.displayOptions.sharpness : 0 },
    pipelinePreference: value.mode === 'video' ? 'video' : 'webgl2',
    superResolutionPreference: value.mode === 'sr' ? 'fsr1Experimental' : 'off',
  }
}

export function renderControlsPatch(value: RenderControlsValue) {
  return { render_mode: value.mode, display_options: { ...value.displayOptions } }
}

export function renderControlsLabel(value: RenderControlsValue, t: (key: string) => string): string {
  if (value.mode === 'video')
    return t('setting.renderControls.defaultRendering')
  if (value.mode === 'sr')
    return 'SR'
  return `${t('setting.renderControls.sharpening')} · ${t(`setting.renderControls.choices.${sharpeningChoice(value)}`)}`
}
