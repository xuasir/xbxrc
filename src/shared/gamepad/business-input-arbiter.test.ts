import type { BusinessInputRouteState } from './business-input-arbiter'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  businessInputArbiter,
  deriveBusinessInputOwner,
  toBusinessInputTracePayload,
} from './business-input-arbiter'
import {
  createBrowserPlayerStreamInputAdapter,
  createRustEngineStreamInputAdapter,
} from './stream-input-consumer-adapters'

function baseState(over: Partial<BusinessInputRouteState> = {}): BusinessInputRouteState {
  return {
    appScene: 'stream',
    backendRoute: 'stream',
    streamActive: true,
    overlayCapturing: false,
    ...over,
  }
}

describe('deriveBusinessInputOwner', () => {
  it('returns none when backendRoute is blocked', () => {
    expect(deriveBusinessInputOwner(baseState({ backendRoute: 'blocked' }))).toBe('none')
  })

  it('returns ui when backendRoute is ui', () => {
    expect(deriveBusinessInputOwner(baseState({ backendRoute: 'ui' }))).toBe('ui')
  })

  it('returns stream when backendRoute is stream', () => {
    expect(deriveBusinessInputOwner(baseState({ backendRoute: 'stream' }))).toBe('stream')
  })

  it('defaults to none when backendRoute is undefined', () => {
    expect(deriveBusinessInputOwner(baseState({ backendRoute: undefined }))).toBe('none')
  })
})

describe('toBusinessInputTracePayload', () => {
  it('projects owner and route state into stable trace fields', () => {
    expect(toBusinessInputTracePayload({
      state: baseState({
        backendRoute: 'ui',
        streamActive: true,
        overlayCapturing: true,
      }),
      owner: 'ui',
    })).toEqual({
      businessInputOwner: 'ui',
      businessInputAppScene: 'stream',
      businessInputBackendRoute: 'ui',
      businessInputStreamActive: true,
      businessInputOverlayCapturing: true,
    })
  })
})

describe('businessInputArbiter', () => {
  afterEach(() => {
    businessInputArbiter.patch({
      appScene: 'shell',
      backendRoute: undefined,
      streamActive: false,
      overlayCapturing: false,
    })
  })

  it('patches partial state', () => {
    businessInputArbiter.patch({
      appScene: 'stream',
      backendRoute: 'ui',
      streamActive: true,
      overlayCapturing: true,
    })
    const state = businessInputArbiter.getState()
    expect(state.appScene).toBe('stream')
    expect(state.backendRoute).toBe('ui')
    expect(state.streamActive).toBe(true)
    expect(state.overlayCapturing).toBe(true)
    expect(businessInputArbiter.getOwner()).toBe('ui')
  })
})

describe('stream input consumer adapters', () => {
  it('activates then deactivates in RFC order', async () => {
    const calls: string[] = []
    const gamepad = {
      setStreamPadForwarding: vi.fn(async (input: { enabled: boolean }) => {
        calls.push(`forward:${input.enabled}`)
      }),
      stopRumble: vi.fn(async () => {
        calls.push('rumble')
      }),
    }
    const adapter = createRustEngineStreamInputAdapter(gamepad)
    await adapter.activateStreamInput()
    await adapter.deactivateStreamInput()
    expect(calls).toEqual(['forward:true', 'rumble', 'forward:false'])
  })

  it('browser adapter is a no-op and does not require RPC side effects', async () => {
    const adapter = createBrowserPlayerStreamInputAdapter()
    await expect(adapter.activateStreamInput()).resolves.toBeUndefined()
    await expect(adapter.deactivateStreamInput()).resolves.toBeUndefined()
  })
})
