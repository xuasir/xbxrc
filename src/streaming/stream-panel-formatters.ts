import type { StreamPerformanceSnapshot, StreamSessionDiagnosticsSnapshot } from './types'

export type StreamPanelTranslate = (key: string, values?: Record<string, unknown>) => string

export function formatPanelMs(value?: number): string {
  if (value === undefined || Number.isNaN(value)) {
    return '--'
  }
  return `${value.toFixed(1)}ms`
}

export function formatPanelKbps(value?: number): string {
  if (value === undefined || value <= 0) {
    return '--'
  }
  if (value >= 1000) {
    return `${(value / 1000).toFixed(1)} Mbps`
  }
  return `${value.toFixed(1)} kbps`
}

export function formatPanelFps(value?: string | number): string {
  if (value === undefined || value === null) {
    return '--'
  }
  const numericValue = Number(value)
  if (Number.isNaN(numericValue)) {
    return '--'
  }
  return numericValue.toFixed(1)
}

export function formatRenderPipelineType(value?: 'video' | 'webgl2'): string {
  if (value === 'video') {
    return 'Video'
  }
  if (value === 'webgl2') {
    return 'WebGL2'
  }
  return '--'
}

export function formatRenderProcessing(value?: 'usm' | 'cas'): string {
  if (value === 'usm') {
    return 'USM'
  }
  if (value === 'cas') {
    return 'CAS'
  }
  return '--'
}

export function formatRenderShaderPath(value?: 'usm' | 'cas' | 'none'): string {
  if (value === 'usm' || value === 'cas' || value === 'none') {
    return value.toUpperCase()
  }
  return '--'
}

export function formatExperienceResolution(input: {
  snapshot: StreamPerformanceSnapshot | null
  resolutionMode?: number
  runtimeMode: 'webrtc-direct' | 'rust-owned'
}): string {
  const resolution = input.snapshot?.resolution
  if (resolution === undefined || resolution === '') {
    return input.runtimeMode === 'webrtc-direct' ? '' : '--'
  }
  return input.resolutionMode === 1081 ? `${resolution}(HQ)` : resolution
}

export function formatSrRuntimeFromSnapshot(
  snapshot: StreamPerformanceSnapshot | null,
  t: StreamPanelTranslate,
): string {
  const s = snapshot
  if (s?.renderSuperResolutionEnabled !== true) {
    return t('streamPage.performance.values.srRunOff')
  }
  if (s.renderSuperResolutionActive === true) {
    const tier = s.renderSuperResolutionOutputTarget ?? s.renderSuperResolutionConfiguredTarget ?? ''
    const alg = (s.renderSuperResolutionAlgorithm ?? 'fsr1').toUpperCase()
    return t('streamPage.performance.values.srRunActive', {
      alg,
      tier: tier !== '' ? String(tier) : '?',
    })
  }
  const reason = s.renderSuperResolutionFallbackReason?.trim()
  if (reason) {
    const short = reason.length > 48 ? `${reason.slice(0, 45)}…` : reason
    return t('streamPage.performance.values.srRunFallback', { reason: short })
  }
  return t('streamPage.performance.values.srRunPending')
}

/**
 * 体验层状态文案：不透传 recoveryOwnerState 等引擎内部枚举串。
 */
export function resolveExperienceStatusText(
  diagnostics: StreamSessionDiagnosticsSnapshot,
  t: StreamPanelTranslate,
): string {
  if (diagnostics.statusCode === 'noVideo') {
    return t('streamPage.diagnostics.values.noVideo')
  }
  if (diagnostics.statusCode === 'probing') {
    return t('streamPage.diagnostics.values.probing')
  }
  if (diagnostics.statusCode === 'recovering') {
    return t('streamPage.diagnostics.values.recovering')
  }
  if (diagnostics.statusCode === 'blocked') {
    return t('streamPage.diagnostics.values.blocked')
  }
  if (diagnostics.statusCode === 'owner') {
    return t('streamPage.experience.statusOwner')
  }
  if (diagnostics.statusCode === 'stable') {
    if (diagnostics.isDisplaySupplyLimited) {
      return t('streamPage.diagnostics.values.displaySupplyLimited')
    }
    return t('streamPage.diagnostics.values.stable')
  }
  return t('streamPage.diagnostics.values.inactive')
}

export function stringOrDash(value?: string | null): string {
  const trimmed = value?.trim()
  return trimmed === undefined || trimmed === '' ? '--' : trimmed
}

export function formatOptionalPercent(value?: number): string {
  if (value === undefined || Number.isNaN(value)) {
    return '--'
  }
  return `${Math.round(value * 100)}%`
}

export function formatBufferedTrend(value?: 'rising' | 'stable' | 'falling'): string {
  if (value === undefined) {
    return '--'
  }
  return value
}
