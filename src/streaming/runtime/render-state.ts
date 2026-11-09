import type { StreamRenderState } from './runtime-contract'

/** 页面只消费已提交管线；准备中的计划独立标记。 */
export function observeRenderState(resources: Record<string, unknown>): StreamRenderState {
  const kind = resources.actualPipeline
  const fallback = resources.actualFallbackReason
  return {
    mode: kind === 'video' ? 'video' : kind === 'webgl2_sr' ? 'sr' : kind === 'webgl2' ? 'sharpen' : null,
    sharpening: kind === 'webgl2' ? Number(resources.actualSharpness) <= 2 ? 'soft' : 'clear' : undefined,
    pending: Number(resources.pendingRenderers ?? 0) > 0,
    fallbackReason: fallback === 'performance' || fallback === 'unsupported' || fallback === 'failed' ? fallback : undefined,
  }
}

export function renderStateLabel(state: StreamRenderState, t: (key: string) => string): string {
  if (state.mode === null)
    return t('setting.renderControls.preparing')
  const label = state.mode === 'video' ? t('setting.renderControls.defaultRendering') : state.mode === 'sr' ? 'SR' : `${t('setting.renderControls.sharpening')} · ${t(`setting.renderControls.choices.${state.sharpening ?? 'clear'}`)}`
  const reason = state.fallbackReason ? t(`setting.renderControls.fallback.${state.fallbackReason}`) : state.pending ? t('setting.renderControls.switching') : ''
  return reason ? `${label} · ${reason}` : label
}
