import type { BusinessInputRouteState } from './business-input-arbiter'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  businessInputArbiter,
  deriveBusinessInputOwner,
  mapStreamRuntimeModeToConsumer,
  selectStreamUiSurfaceFromPageFlags,
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
    streamSessionId: 's1',
    streamSessionPresent: true,
    streamConsumer: 'browser-player',
    streamUiSurface: 'none',
    rustEngineStreamPadRoutedToSession: true,
    chromeVisible: true,
    ...over,
  }
}

describe('deriveBusinessInputOwner', () => {
  it('returns none when backend gate is closed', () => {
    expect(deriveBusinessInputOwner(baseState({ backendGate: 'closed' }))).toBe('none')
  })

  it('returns ui on shell scene even when session present', () => {
    expect(deriveBusinessInputOwner(baseState({ appScene: 'shell' }))).toBe('ui')
  })

  it('returns ui when stream session not present', () => {
    expect(deriveBusinessInputOwner(baseState({ streamSessionPresent: false }))).toBe('ui')
  })

  it('returns ui when any stream UI surface is open', () => {
    expect(deriveBusinessInputOwner(baseState({ streamUiSurface: 'menu' }))).toBe('ui')
    expect(deriveBusinessInputOwner(baseState({ streamUiSurface: 'failed' }))).toBe('ui')
  })

  it('returns stream when playing in browser mode with surface none', () => {
    expect(deriveBusinessInputOwner(baseState({
      streamConsumer: 'browser-player',
      streamUiSurface: 'none',
    }))).toBe('stream')
  })

  it('keeps ui for rust-engine until pad route reports stream-session applied', () => {
    expect(deriveBusinessInputOwner(baseState({
      streamConsumer: 'rust-engine',
      streamUiSurface: 'none',
      rustEngineStreamPadRoutedToSession: false,
    }))).toBe('ui')
    expect(deriveBusinessInputOwner(baseState({
      streamConsumer: 'rust-engine',
      streamUiSurface: 'none',
      rustEngineStreamPadRoutedToSession: true,
    }))).toBe('stream')
  })

  it('does not use chromeVisible alone to flip owner', () => {
    expect(deriveBusinessInputOwner(baseState({
      chromeVisible: true,
      streamUiSurface: 'none',
      streamConsumer: 'browser-player',
    }))).toBe('stream')
  })
})

describe('selectStreamUiSurfaceFromPageFlags', () => {
  const none = {
    showFailedSheet: false,
    showWarningSheet: false,
    isMenuSheetOpen: false,
    isDiagnosticsMenuSheetOpen: false,
    isDisplaySheetOpen: false,
    isAudioSheetOpen: false,
    isTextSheetOpen: false,
  }

  it('prioritizes failed over menu', () => {
    expect(selectStreamUiSurfaceFromPageFlags({
      ...none,
      showFailedSheet: true,
      isMenuSheetOpen: true,
    })).toBe('failed')
  })

  it('prioritizes warning over menu', () => {
    expect(selectStreamUiSurfaceFromPageFlags({
      ...none,
      showWarningSheet: true,
      isMenuSheetOpen: true,
    })).toBe('warning')
  })

  it('orders menu before diagnostics before sheets', () => {
    expect(selectStreamUiSurfaceFromPageFlags({ ...none, isMenuSheetOpen: true })).toBe('menu')
    expect(selectStreamUiSurfaceFromPageFlags({
      ...none,
      isDiagnosticsMenuSheetOpen: true,
    })).toBe('diagnosticsMenu')
    expect(selectStreamUiSurfaceFromPageFlags({ ...none, isDisplaySheetOpen: true })).toBe('display')
    expect(selectStreamUiSurfaceFromPageFlags({ ...none, isAudioSheetOpen: true })).toBe('audio')
    expect(selectStreamUiSurfaceFromPageFlags({ ...none, isTextSheetOpen: true })).toBe('text')
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

describe('mapStreamRuntimeModeToConsumer', () => {
  it('maps runtime modes', () => {
    expect(mapStreamRuntimeModeToConsumer('webrtc-direct')).toBe('browser-player')
    expect(mapStreamRuntimeModeToConsumer('rust-owned')).toBe('rust-engine')
    expect(mapStreamRuntimeModeToConsumer('other')).toBe('none')
  })
})

describe('toBusinessInputTracePayload', () => {
  it('projects owner and route state into stable trace fields', () => {
    expect(toBusinessInputTracePayload({
      state: baseState({
        appScene: 'stream',
        backendGate: 'open',
        streamSessionId: 'trace-session',
        streamSessionPresent: true,
        streamConsumer: 'rust-engine',
        streamUiSurface: 'menu',
        rustEngineStreamPadRoutedToSession: false,
        chromeVisible: true,
      }),
      owner: 'ui',
    })).toEqual({
      businessInputOwner: 'ui',
      businessInputAppScene: 'stream',
      businessInputBackendGate: 'open',
      businessInputStreamSessionPresent: true,
      businessInputStreamSessionId: 'trace-session',
      businessInputStreamConsumer: 'rust-engine',
      businessInputStreamUiSurface: 'menu',
      businessInputRustStreamSessionRouted: false,
      businessInputChromeVisible: true,
    })
  })
})

describe('applyActionOutcome', () => {
  afterEach(() => {
    businessInputArbiter.patch({
      appScene: 'shell',
      backendGate: 'open',
      streamSessionId: null,
      streamSessionPresent: false,
      streamConsumer: 'none',
      streamUiSurface: 'none',
      rustEngineStreamPadRoutedToSession: false,
      chromeVisible: false,
    })
  })

  it('stay-ui updates optional surface', () => {
    businessInputArbiter.patch({ streamUiSurface: 'none' })
    businessInputArbiter.applyActionOutcome({ kind: 'stay-ui', nextSurface: 'display' })
    expect(businessInputArbiter.getState().streamUiSurface).toBe('display')
  })

  it('resume-stream clears surface', () => {
    businessInputArbiter.patch({
      appScene: 'stream',
      streamSessionPresent: true,
      streamUiSurface: 'menu',
      streamConsumer: 'browser-player',
      rustEngineStreamPadRoutedToSession: true,
    })
    businessInputArbiter.applyActionOutcome({ kind: 'resume-stream' })
    expect(businessInputArbiter.getState().streamUiSurface).toBe('none')
  })

  it('leave-stream clears session fields', () => {
    businessInputArbiter.patch({
      appScene: 'stream',
      streamSessionPresent: true,
      streamSessionId: 'x',
      streamConsumer: 'rust-engine',
      rustEngineStreamPadRoutedToSession: true,
    })
    businessInputArbiter.applyActionOutcome({ kind: 'leave-stream' })
    const s = businessInputArbiter.getState()
    expect(s.streamSessionPresent).toBe(false)
    expect(s.streamSessionId).toBeNull()
    expect(s.streamConsumer).toBe('none')
    expect(s.rustEngineStreamPadRoutedToSession).toBe(false)
  })
})

describe('stream input consumer adapters', () => {
  it('activates then deactivates in RFC order', async () => {
    const calls: string[] = []
    const gamepad = {
      setStreamPadForwarding: vi.fn(async (input: { enabled: boolean }) => {
        const enabled = input.enabled
        calls.push(`forward:${enabled}`)
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

describe('applyStreamPadRouteTarget', () => {
  afterEach(() => {
    businessInputArbiter.patch({
      appScene: 'shell',
      backendGate: 'open',
      streamSessionId: null,
      streamSessionPresent: false,
      streamConsumer: 'none',
      streamUiSurface: 'none',
      rustEngineStreamPadRoutedToSession: false,
      chromeVisible: false,
    })
  })

  it('uses installed adapter rather than assuming rust forwarding', async () => {
    const calls: string[] = []
    businessInputArbiter.installStreamInputConsumerAdapter({
      activateStreamInput: vi.fn(async () => {
        calls.push('activate')
      }),
      deactivateStreamInput: vi.fn(async () => {
        calls.push('deactivate')
      }),
    })
    await businessInputArbiter.applyStreamPadRouteTarget({ kind: 'stream-session' })
    await businessInputArbiter.applyStreamPadRouteTarget({ kind: 'shell-ui' })
    expect(calls).toEqual(['activate', 'deactivate'])
  })
})
