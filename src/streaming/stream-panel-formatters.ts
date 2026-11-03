import type { StreamPerformanceSnapshot, StreamSessionDiagnosticsSnapshot } from './types'

export type StreamPanelTranslate = (key: string, values?: Record<string, unknown>) => string

export function formatPanelMs(value?: number): string {
  if (value === undefined || !Number.isFinite(value)) {
    return '--'
  }
  return `${value.toFixed(1)}ms`
}

export function formatPanelKbps(value?: number): string {
  if (value === undefined || !Number.isFinite(value) || value < 0) {
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
  if (!Number.isFinite(numericValue) || numericValue < 0) {
    return '--'
  }
  return numericValue.toFixed(1)
}

/** 运行条使用数值事实；兼容旧的带单位字符串，同时过滤启动占位值。 */
export function panelNumericValue(value?: string | number): number | undefined {
  if (value === undefined || String(value).trim() === '')
    return undefined
  const numeric = typeof value === 'number' ? value : Number.parseFloat(value)
  return Number.isFinite(numeric) && numeric >= 0 ? numeric : undefined
}

export function formatRuntimeResolution(snapshot: StreamPerformanceSnapshot | null): string {
  const dimensions = snapshot?.resolution?.match(/^(\d+)\s*[x×]\s*(\d+)/i)
  const width = snapshot?.renderSourceWidth ?? (dimensions ? Number(dimensions[1]) : undefined)
  const height = snapshot?.renderSourceHeight ?? (dimensions ? Number(dimensions[2]) : undefined)
  if (!width || !height)
    return '--'
  const standard: Record<number, number> = { 720: 1280, 1080: 1920, 1440: 2560, 2160: 3840 }
  return standard[height] === width ? `${height}p` : formatPanelDimensions(width, height)
}

export function formatRuntimeLoss(snapshot: StreamPerformanceSnapshot | null): string {
  const ratio = snapshot?.networkPacketLossRatio
  const percent = typeof ratio === 'number' && Number.isFinite(ratio) && ratio >= 0 && ratio <= 1
    ? ratio * 100
    : panelNumericValue(String(snapshot?.pl ?? '').match(/(-?\d+(?:\.\d+)?)\s*%/)?.[1])
  return percent === undefined || percent > 100 ? '--' : `${percent.toFixed(1)}%`
}

export function formatPanelDimensions(width?: number, height?: number): string {
  return width && height ? `${Math.round(width)}×${Math.round(height)}` : '--'
}

export function formatRenderPipelineType(value?: 'video' | 'webgl2' | 'webgl2_sr'): string {
  if (value === 'video') {
    return 'Video'
  }
  if (value === 'webgl2') {
    return 'WebGL2'
  }
  if (value === 'webgl2_sr') {
    return 'WebGL2 + SR'
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
