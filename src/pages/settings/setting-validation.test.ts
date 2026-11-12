import { CONFIG_FIELD_DEFINITIONS } from '@shared/config/domain-definition'
import { describe, expect, it } from 'vitest'
import { getSectionsForPage, getSettingEffect, isSettingFieldVisible } from './setting-page-schema'
import { validateCustomTurn, validateSettingNumber } from './setting-validation'

describe('settings numeric contract', () => {
  it.each(['', ' ', '\n'])('rejects empty input %j', value => expect(validateSettingNumber(value)).toBe('required'))
  it.each(['NaN', 'Infinity', 'x'])('rejects non-finite input %s', value => expect(validateSettingNumber(value)).toBe('number'))
  it('validates range and steps against the Rust audio bitrate limit', () => {
    const input = CONFIG_FIELD_DEFINITIONS.audio_bitrate.input
    expect(input?.max).toBe(200)
    expect(validateSettingNumber('200', input)).toBeNull()
    expect(validateSettingNumber('201', input)).toBe('range')
    expect(validateSettingNumber('-1', input)).toBe('range')
    expect(validateSettingNumber('1.5', input)).toBe('step')
    expect(validateSettingNumber('0.3', { min: 0.1, step: 0.1 })).toBeNull()
  })
})

describe('custom TURN configuration', () => {
  const draft = { server_url: 'turn:relay.example.com:3478', server_username: 'user', server_credential: 'secret' }
  it.each(['server_url', 'server_username', 'server_credential'])('requires %s', key => expect(validateCustomTurn({ ...draft, [key]: ' ' })).toBe('incomplete'))
  it.each(['turn:relay.example.com:3478', 'turns:relay.example.com:5349?transport=tcp', 'turn:[2001:db8::1]:3478?transport=udp'])('accepts %s', server_url => expect(validateCustomTurn({ ...draft, server_url })).toBeNull())
  it.each(['https://relay.example.com', 'turn://relay.example.com', 'turn:relay.example.com/', 'turn:relay example.com', 'turn:', 'turn:user:secret@relay.example.com', 'turn:relay.example.com/path', 'turn:relay.example.com?transport=http', 'turn:relay.example.com#token'])('rejects %s', server_url => expect(validateCustomTurn({ ...draft, server_url })).toBe('url'))
})

describe('settings organization and effect', () => {
  it('shows manual bitrate only when its own mode is Custom', () => {
    const groups = { streaming: { xhome_bitrate_mode: 'Auto', xcloud_bitrate_mode: 'Custom', audio_bitrate_mode: 'Auto' } }
    expect(isSettingFieldVisible(groups, 'xhome_bitrate')).toBe(false)
    expect(isSettingFieldVisible(groups, 'xcloud_bitrate')).toBe(true)
    expect(isSettingFieldVisible(groups, 'audio_bitrate')).toBe(false)
    expect(isSettingFieldVisible(groups, 'resolution')).toBe(true)
  })
  it('reports authorization, connection, launch, and immediate setting effects', () => {
    expect(getSettingEffect('fullscreen')).toBe('nextLaunch')
    expect(getSettingEffect('resolution')).toBe('nextConnection')
    expect(getSettingEffect('force_region_ip')).toBe('nextAuthorization')
    expect(getSettingEffect('xcloud_region_name')).toBe('nextConnection')
    expect(getSettingEffect('theme')).toBe('instant')
    expect(getSettingEffect('runtime_trace_mode')).toBe('instant')
    expect(getSectionsForPage('general').flatMap(section => section.items)).not.toContainEqual({ kind: 'field', fieldKey: 'background_keepalive' })
    expect(getSectionsForPage('connectionHost').at(-1)?.items).toContainEqual({ kind: 'action', actionId: 'customTurn' })
  })
})
