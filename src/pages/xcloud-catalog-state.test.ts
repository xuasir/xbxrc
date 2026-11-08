import { describe, expect, it } from 'vitest'
import { CatalogRequestState } from './xcloud-catalog-state'

describe('cloud catalog request scope', () => {
  it('discards late responses and old account events after authentication changes', () => {
    const state = new CatalogRequestState()
    const oldRequest = state.capture()
    expect(state.acceptResponse(oldRequest, { scopeKey: 'account-a-region', updatedAt: 1 })).toBe(true)
    state.clear()
    expect(state.acceptResponse(oldRequest, { scopeKey: 'account-a-region', updatedAt: 2 })).toBe(false)
    expect(state.acceptEvent({ scopeKey: 'account-a-region', updatedAt: 2 })).toBe(false)
    expect(state.acceptResponse(state.capture(), { scopeKey: 'account-b-region', updatedAt: 3 })).toBe(true)
    expect(state.acceptEvent({ scopeKey: 'account-a-region', updatedAt: 4 })).toBe(false)
    expect(state.acceptEvent({ scopeKey: 'account-b-region', updatedAt: 4 })).toBe(true)
  })

  it('keeps a newer background refresh when an older RPC or event arrives later', () => {
    const state = new CatalogRequestState()
    expect(state.acceptResponse(state.capture(), { scopeKey: 'account-region', updatedAt: 10 })).toBe(true)
    expect(state.acceptEvent({ scopeKey: 'account-region', updatedAt: 20 })).toBe(true)
    expect(state.acceptResponse(state.capture(), { scopeKey: 'account-region', updatedAt: 10 })).toBe(false)
    expect(state.acceptEvent({ scopeKey: 'account-region', updatedAt: 19 })).toBe(false)
    expect(state.acceptEvent({ scopeKey: 'account-region' })).toBe(false)
    expect(state.acceptEvent({ scopeKey: 'account-other-language', updatedAt: 30 })).toBe(false)
  })
})
