/**
 * 引擎下发的诊断枚举（camelCase / kebab-case）映射为 i18n 可读文案。
 * 无词条时回退原文，避免空白。
 */
export type DiagnosticsTranslate = (key: string) => string
export type DiagnosticsExists = (key: string) => boolean

const BASE = 'streamPage.diagnostics.enums'

function translateEnum(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  subPath: string,
  raw: string | undefined | null,
  emptyKey: string,
): string {
  if (raw === undefined || raw === null || raw.trim() === '') {
    return t(emptyKey)
  }
  const key = `${BASE}.${subPath}.${raw}`
  return te(key) ? t(key) : raw
}

/** videoHealth：priming / healthy / recovering / displaySupplyStarved … */
export function translateDiagnosticsVideoHealth(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  raw?: string | null,
): string {
  return translateEnum(te, t, 'videoHealth', raw, 'streamPage.diagnostics.values.unknown')
}

/** stallKind：none / waitingKeyframe / displaySupplyStarved … */
export function translateDiagnosticsStallKind(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  raw?: string | null,
): string {
  return translateEnum(te, t, 'stallKind', raw, 'streamPage.diagnostics.values.none')
}

/** sessionPhase：connecting / handshaking / priming / steady / recovering */
export function translateDiagnosticsSessionPhase(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  raw?: string | null,
): string {
  return translateEnum(te, t, 'sessionPhase', raw, 'streamPage.diagnostics.values.unknown')
}

/** recoveryOwnerState：stable-serving / supply-starved … */
export function translateDiagnosticsOwnerState(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  raw?: string | null,
): string {
  return translateEnum(te, t, 'ownerState', raw, 'streamPage.diagnostics.values.unknown')
}

/** recoveryOwnerReason：transportAwaitRecoveryKeyframe / supplyStarved … */
export function translateDiagnosticsOwnerReason(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  raw?: string | null,
): string {
  return translateEnum(te, t, 'ownerReason', raw, 'streamPage.diagnostics.values.none')
}

/** 解码器恢复态：nominal 等 */
export function translateDiagnosticsDecoderRecovery(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  raw?: string | null,
): string {
  return translateEnum(te, t, 'decoderRecovery', raw, 'streamPage.diagnostics.values.unknown')
}

/**
 * primaryIssueChain：steady:healthy、display:supplyStarved、recovery:transportAwaitRecoveryKeyframe …
 * 优先整串词条；否则按「前缀 · 细节」拼接翻译。
 */
export function translateDiagnosticsPrimaryIssueChain(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  raw?: string | null,
): string {
  if (raw === undefined || raw === null || raw.trim() === '') {
    return t('streamPage.diagnostics.values.unknown')
  }
  const flatKey = `${BASE}.primaryIssueChain.${raw.replace(/:/g, '__')}`
  if (te(flatKey)) {
    return t(flatKey)
  }
  const colon = raw.indexOf(':')
  if (colon <= 0) {
    return raw
  }
  const prefix = raw.slice(0, colon)
  const detail = raw.slice(colon + 1)
  const pKey = `${BASE}.issuePrefix.${prefix}`
  const dKey = `${BASE}.issueDetail.${detail.replace(/:/g, '__')}`
  const p = te(pKey) ? t(pKey) : prefix
  const d = te(dKey) ? t(dKey) : detail
  return `${p} · ${d}`
}

/**
 * latestDecisionSummary：owner:rebuilding-supply:transportAwaitRecoveryKeyframe …
 */
export function translateDiagnosticsLatestDecision(
  te: DiagnosticsExists,
  t: DiagnosticsTranslate,
  raw?: string | null,
): string {
  if (raw === undefined || raw === null || raw.trim() === '') {
    return t('streamPage.diagnostics.values.none')
  }
  const flatKey = `${BASE}.latestDecision.${raw.replace(/:/g, '__')}`
  return te(flatKey) ? t(flatKey) : raw
}
