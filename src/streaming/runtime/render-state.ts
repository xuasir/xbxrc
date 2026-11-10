import type { StreamRenderState } from './runtime-contract'
import { COLOR_PRESETS } from '../../shared/config/render-controls'

/** 页面只消费已提交管线；准备中的计划独立标记。 */
export function observeRenderState(resources: Record<string, unknown>): StreamRenderState {
  const kind = resources.actualPipeline
  const fallback = resources.actualFallbackReason
  const canvas = kind === 'webgl2' || kind === 'webgl2_sr'
  const colors = { brightness: resources.actualBrightness ?? 100, contrast: resources.actualContrast ?? 100, saturation: resources.actualSaturation ?? 100 }
  const style = (Object.keys(COLOR_PRESETS) as (keyof typeof COLOR_PRESETS)[]).find(key => Object.entries(COLOR_PRESETS[key]).every(([name, value]) => colors[name as keyof typeof colors] === value))
  return {
    engine: kind === 'video' ? 'compatibility' : canvas ? 'passthrough' : null,
    superResolution: canvas && kind === 'webgl2_sr',
    superResolutionActive: typeof resources.actualSuperResolutionActive === 'boolean' ? resources.actualSuperResolutionActive : undefined,
    sharpening: canvas && Number(resources.actualSharpness) > 0 ? Number(resources.actualSharpness) <= 2 ? 'soft' : 'clear' : undefined,
    style: canvas ? style ?? 'custom' : undefined,
    pending: Number(resources.pendingRenderers ?? 0) > 0,
    fallbackReason: fallback === 'performance' || fallback === 'unsupported' || fallback === 'failed' ? fallback : undefined,
  }
}

export function renderStateLabel(state: StreamRenderState, t: (key: string) => string): string {
  if (state.engine === null)
    return t('setting.renderControls.preparing')
  const parts: string[] = state.engine === 'compatibility' ? [t('setting.renderControls.compatibilityRendering')] : []
  if (state.superResolution)
    parts.push(state.superResolutionActive === false ? t('setting.renderControls.srAtTarget') : 'SR')
  if (state.sharpening)
    parts.push(`${t('setting.renderControls.sharpening')}${t(`setting.renderControls.choices.${state.sharpening}`)}`)
  if (state.style && state.style !== 'standard')
    parts.push(t(`setting.renderControls.colors.${state.style}`))
  const label = parts.length ? parts.join(' · ') : t('setting.renderControls.defaultRendering')
  const reason = state.fallbackReason ? t(`setting.renderControls.fallback.${state.fallbackReason}`) : state.pending ? t('setting.renderControls.switching') : ''
  return reason ? `${label} · ${reason}` : label
}
