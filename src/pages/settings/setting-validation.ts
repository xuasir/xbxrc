import type { SettingFieldInputDefinition } from '@shared/config/domain-definition'

export type NumberValidationError = 'required' | 'number' | 'range' | 'step'

export function validateSettingNumber(raw: string, input: SettingFieldInputDefinition = {}): NumberValidationError | null {
  if (raw.trim() === '')
    return 'required'
  const value = Number(raw)
  if (!Number.isFinite(value))
    return 'number'
  if ((input.min !== undefined && value < input.min) || (input.max !== undefined && value > input.max))
    return 'range'
  if (input.step && Math.abs((value - (input.min ?? 0)) / input.step - Math.round((value - (input.min ?? 0)) / input.step)) > 1e-8)
    return 'step'
  return null
}

export interface CustomTurnDraft {
  server_url: string
  server_username: string
  server_credential: string
}

export function validateCustomTurn(value: CustomTurnDraft): 'incomplete' | 'url' | null {
  const url = value.server_url.trim()
  if (!url || !value.server_username.trim() || !value.server_credential.trim())
    return 'incomplete'
  if (!/^turns?:[^/?#\s]+(?:\?transport=(?:udp|tcp))?$/i.test(url))
    return 'url'
  try {
    // TURN URI 使用 host:port，接受安全传输与标准 transport 查询参数。
    const parsed = new URL(url.replace(/^turns?:/i, 'https://'))
    if (!/^turns?:/i.test(url) || !parsed.hostname || parsed.username || parsed.password || parsed.hash || (parsed.pathname && parsed.pathname !== '/') || (parsed.search && !/^\?transport=(?:udp|tcp)$/i.test(parsed.search)))
      return 'url'
  }
  catch {
    return 'url'
  }
  return null
}
