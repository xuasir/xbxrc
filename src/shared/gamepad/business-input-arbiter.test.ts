import type { BusinessInputRouteState } from './business-input-arbiter'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  businessInputArbiter,
  deriveBusinessInputOwner,
  snapshotGateToBackendGate,
  toBusinessInputTracePayload,
} from './business-input-arbiter'
import {
  createBrowserPlayerStreamInputAdapter,
  createRustEngineStreamInputAdapter,
} from './stream-input-consumer-adapters'

function baseState(over: Partial<BusinessInputRouteState> = {}): BusinessInputRouteState {
  return {
    appScene: 'stream',
    backendGate: 'open',
    streamActive: true,
    overlayCapturing: false,
    ...over,
  }
}

describe('deriveBusinessInputOwner', () => {
  it('returns none when backend gate is closed', () => {
    expect(deriveBusinessInputOwner(baseState({ backendGate: 'closed' }))).toBe('none')
  })

  it('returns ui on shell scene even when stream active', () => {
    expect(deriveBusinessInputOwner(baseState({ appScene: 'shell' }))).toBe('ui')
  })

  it('returns ui when stream session not active', () => {
    expect(deriveBusinessInputOwner(baseState({ streamActive: false }))).toBe('ui')
  })

  it('returns ui when overlay is capturing', () => {
    expect(deriveBusinessInputOwner(baseState({ overlayCapturing: true }))).toBe('ui')
  })

  it('returns stream when stream active and overlay not capturing', () => {
    expect(deriveBusinessInputOwner(baseState())).toBe('stream')
  })
})

describe('snapshotGateToBackendGate', () => {
  it('maps closed', () => {
    expect(snapshotGateToBackendGate('closed')).toBe('closed')
  })
  it('defaults open', () => {
    expect(snapshotGateToBackendGate(undefined)).toBe('open')
    expect(snapshotGateToBackendGate('open')).toBe('open')
  })
})

describe('toBusinessInputTracePayload', () => {
  it('projects owner and route state into stable trace fields', () => {
    expect(toBusinessInputTracePayload({
      state: baseState({
        appScene: 'stream',
        backendGate: 'open',
        streamActive: true,
        overlayCapturing: true,
      }),
      owner: 'ui',
    })).toEqual({
      businessInputOwner: 'ui',
      businessInputAppScene: 'stream',
      businessInputBackendGate: 'open',
      businessInputStreamActive: true,
      businessInputOverlayCapturing: true,
    })
  })
})

describe('businessInputArbiter', () => {
  afterEach(() => {
    businessInputArbiter.patch({
      appScene: 'shell',
      backendGate: 'open',
      streamActive: false,
      overlayCapturing: false,
    })
  })

  it('patches partial state', () => {
    businessInputArbiter.patch({
      appScene: 'stream',
      streamActive: true,
      overlayCapturing: true,
    })
    const state = businessInputArbiter.getState()
    expect(state.appScene).toBe('stream')
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
